//! A history-owned staging copy of one asset's
//! bytes. The lease pins the bytes an undo or a save snapshot still needs
//! — the staged file lives exactly as long as the last `Arc` holder, so
//! eviction of a history entry, a project close, or the last job finishing
//! all release what nothing references anymore. Regenerable cache bytes
//! never come through here.
use crate::{AssetId, Result, store::AssetMetaOrigin};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_STAGED_PAYLOAD_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Staged bytes of one asset, owned by the project's history.
#[derive(Debug)]
pub struct PayloadLease {
    asset: AssetId,
    sha256: String,
    size: u64,
    path: PathBuf,
    origin: Option<AssetMetaOrigin>,
    /// Keeps a session-owned temporary directory alive while a snapshot/job
    /// still holds the payload after the session closes.
    _staging_owner: Option<Arc<tempfile::TempDir>>,
}

impl PayloadLease {
    /// Stage `source` in `staging` and lease it. Same-filesystem sources can
    /// be hard-linked (application-owned files only); either way, declared
    /// size and SHA-256 must match.
    #[cfg(test)]
    pub(crate) fn stage(
        staging: &Path,
        asset: AssetId,
        sha256: &str,
        size: u64,
        source: &Path,
        origin: Option<AssetMetaOrigin>,
        staging_owner: Option<Arc<tempfile::TempDir>>,
    ) -> Result<Arc<Self>> {
        Self::stage_cancellable(
            staging,
            asset,
            sha256,
            size,
            source,
            origin,
            staging_owner,
            true,
            || false,
        )
    }

    // Staging takes the lease identity, the source, its owner and the cancel hook as distinct inputs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stage_cancellable(
        staging: &Path,
        asset: AssetId,
        sha256: &str,
        size: u64,
        source: &Path,
        origin: Option<AssetMetaOrigin>,
        staging_owner: Option<Arc<tempfile::TempDir>>,
        // Only for application-owned files: a link shares later in-place writes.
        link_source: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<Arc<Self>> {
        if cancelled() {
            return Err(crate::ProjectError::Cancelled);
        }
        if size > MAX_STAGED_PAYLOAD_BYTES {
            return Err(crate::ProjectError::ResourceLimit(
                "asset exceeds 4 GiB".into(),
            ));
        }
        std::fs::create_dir_all(staging)?;
        let path = staging.join(format!("{}.payload", uuid::Uuid::new_v4()));
        if link_source && std::fs::hard_link(source, &path).is_ok() {
            let mut pending = PendingPayloadFile::new(path.clone());
            match verify_staged_file(&path, size, sha256, &cancelled) {
                Ok(()) => {
                    pending.keep();
                    return Ok(Arc::new(Self {
                        asset,
                        sha256: sha256.to_owned(),
                        size,
                        path,
                        origin,
                        _staging_owner: staging_owner,
                    }));
                }
                Err(crate::ProjectError::Cancelled) => {
                    return Err(crate::ProjectError::Cancelled);
                }
                Err(_) => {}
            }
            // A failed verification must not leave the link in place. Retry
            // through the copy path so its established error mapping remains
            // unchanged (and so a transient link/read failure can recover).
            drop(pending);
        }
        let mut input = std::fs::File::open(source)?;
        let mut pending = PendingPayloadFile::new(path.clone());
        let mut output = std::fs::File::create(&path)?;
        let mut hasher = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            if cancelled() {
                return Err(crate::ProjectError::Cancelled);
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            copied = copied
                .checked_add(count as u64)
                .ok_or_else(|| crate::ProjectError::ResourceLimit("asset size overflow".into()))?;
            if copied > size || copied > MAX_STAGED_PAYLOAD_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("staged copy exceeds declared size {size}"),
                )
                .into());
            }
            hasher.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
        if copied != size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "staged copy has {copied} bytes, {} declared {size}",
                    source.display()
                ),
            )
            .into());
        }
        if format!("{:x}", hasher.finalize()) != sha256 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "staged asset bytes do not match the declared SHA-256",
            )
            .into());
        }
        drop(output);
        pending.keep();
        Ok(Arc::new(Self {
            asset,
            sha256: sha256.to_owned(),
            size,
            path,
            origin,
            _staging_owner: staging_owner,
        }))
    }

    /// Stage `source` with `patches` (byte offset, replacement) applied in
    /// one pass. The source must still hold the recorded bytes (`size`,
    /// `expected_sha256`), else an `InvalidData` error the caller names;
    /// the lease records the patched bytes' hash. Patches may come in any order
    /// but must be disjoint and inside the source.
    // Staging takes the lease identity, the source, its owner and the cancel hook as distinct inputs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stage_patched_cancellable(
        staging: &Path,
        asset: AssetId,
        source: &Path,
        size: u64,
        expected_sha256: &str,
        patches: &[(u64, Vec<u8>)],
        staging_owner: Option<Arc<tempfile::TempDir>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Arc<Self>> {
        if size > MAX_STAGED_PAYLOAD_BYTES {
            return Err(crate::ProjectError::ResourceLimit(
                "asset exceeds 4 GiB".into(),
            ));
        }
        let mut sorted = patches.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|(start, _)| *start);
        let mut end = 0u64;
        for (start, bytes) in sorted {
            if *start < end || start.saturating_add(bytes.len() as u64) > size {
                return Err(crate::ProjectError::Invalid(
                    "patches must be disjoint and inside the asset".into(),
                ));
            }
            end = start + bytes.len() as u64;
        }
        std::fs::create_dir_all(staging)?;
        let path = staging.join(format!("{}.payload", uuid::Uuid::new_v4()));
        let mut input = std::fs::File::open(source)?;
        let mut pending = PendingPayloadFile::new(path.clone());
        let mut output = std::fs::File::create(&path)?;
        let (mut old_hash, mut new_hash) = (Sha256::new(), Sha256::new());
        let mut copied = 0u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            if cancelled() {
                return Err(crate::ProjectError::Cancelled);
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let block = copied..copied + count as u64;
            copied = block.end;
            if copied > size {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("staged copy exceeds declared size {size}"),
                )
                .into());
            }
            old_hash.update(&buffer[..count]);
            for (start, bytes) in patches {
                let patch = *start..start + bytes.len() as u64;
                let (from, to) = (patch.start.max(block.start), patch.end.min(block.end));
                if from < to {
                    buffer[(from - block.start) as usize..(to - block.start) as usize]
                        .copy_from_slice(
                            &bytes[(from - patch.start) as usize..(to - patch.start) as usize],
                        );
                }
            }
            new_hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        if copied != size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "staged copy has {copied} bytes, {} declared {size}",
                    source.display()
                ),
            )
            .into());
        }
        if format!("{:x}", old_hash.finalize()) != expected_sha256 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "staged asset bytes do not match the declared SHA-256",
            )
            .into());
        }
        output.sync_all()?;
        drop(output);
        pending.keep();
        Ok(Arc::new(Self {
            asset,
            sha256: format!("{:x}", new_hash.finalize()),
            size,
            path,
            origin: None,
            _staging_owner: staging_owner,
        }))
    }

    /// The asset whose bytes this lease stages.
    pub fn asset(&self) -> AssetId {
        self.asset
    }

    /// The content hash the model records for these bytes.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The staged byte count.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The staged file; read by the store's save when re-materializing.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Persisted import metadata needed to reconstruct the asset sidecar.
    pub(crate) fn origin(&self) -> Option<&AssetMetaOrigin> {
        self.origin.as_ref()
    }

    /// The same bytes leased from `staging`. A staged file is never written
    /// after staging, so a link (or a copy across filesystems) needs no
    /// second verification.
    pub(crate) fn relink(
        &self,
        staging: &Path,
        staging_owner: Option<Arc<tempfile::TempDir>>,
    ) -> Result<Arc<Self>> {
        let path = staging.join(format!("{}.payload", uuid::Uuid::new_v4()));
        let mut pending = PendingPayloadFile::new(path.clone());
        if std::fs::hard_link(&self.path, &path).is_err() {
            std::fs::copy(&self.path, &path)?;
        }
        pending.keep();
        Ok(Arc::new(Self {
            asset: self.asset,
            sha256: self.sha256.clone(),
            size: self.size,
            path,
            origin: self.origin.clone(),
            _staging_owner: staging_owner,
        }))
    }
}

fn verify_staged_file(
    path: &Path,
    size: u64,
    sha256: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    let mut input = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut read = 0u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancelled() {
            return Err(crate::ProjectError::Cancelled);
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        read = read
            .checked_add(count as u64)
            .ok_or_else(|| crate::ProjectError::ResourceLimit("asset size overflow".into()))?;
        if read > size || read > MAX_STAGED_PAYLOAD_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("staged copy exceeds declared size {size}"),
            )
            .into());
        }
        hasher.update(&buffer[..count]);
    }
    if read != size {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "staged copy has {read} bytes, {} declared {size}",
                path.display()
            ),
        )
        .into());
    }
    if format!("{:x}", hasher.finalize()) != sha256 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "staged asset bytes do not match the declared SHA-256",
        )
        .into());
    }
    Ok(())
}

struct PendingPayloadFile {
    path: Option<PathBuf>,
}

impl PendingPayloadFile {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn keep(&mut self) {
        self.path = None;
    }
}

impl Drop for PendingPayloadFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for PayloadLease {
    fn drop(&mut self) {
        // The lease created this file inside the staging area; releasing
        // The last holder removes the file when nothing references it anymore.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_leases_own_their_staged_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let source = temporary.path().join("mesh.ply");
        std::fs::write(&source, b"ply bytes").unwrap();
        let asset = AssetId::new();
        let lease = PayloadLease::stage(
            &staging,
            asset,
            &sha256hex(b"ply bytes"),
            b"ply bytes".len() as u64,
            &source,
            None,
            None,
        )
        .unwrap();
        assert_eq!(lease.asset(), asset);
        assert_eq!(lease.size(), b"ply bytes".len() as u64);
        let staged = std::fs::read(lease.path()).unwrap();
        assert_eq!(staged, b"ply bytes", "the lease holds a real copy");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(source).unwrap().ino(),
                std::fs::metadata(lease.path()).unwrap().ino(),
                "same-filesystem staging reuses the source inode"
            );
        }
        // A shared lease keeps the bytes alive for every holder.
        let shared = lease.clone();
        drop(lease);
        assert_eq!(std::fs::read(shared.path()).unwrap(), b"ply bytes");
        // The last holder releases the staging file.
        let path = shared.path().to_path_buf();
        drop(shared);
        assert!(
            !path.is_file(),
            "releasing the last lease removes the staged copy"
        );
        assert!(
            std::fs::read_dir(&staging).unwrap().next().is_none(),
            "nothing is left behind in the staging area"
        );
    }

    /// A source that does not match its declared size or hash is invalid data.
    #[test]
    fn a_source_unlike_its_record_is_invalid_data() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("staging");
        let source = temporary.path().join("image.png");
        std::fs::write(&source, b"image bytes plus").unwrap();
        for (sha256, size) in [
            (sha256hex(b"image bytes"), b"image bytes".len() as u64),
            (
                sha256hex(b"other bytes plus"),
                b"image bytes plus".len() as u64,
            ),
        ] {
            let error =
                PayloadLease::stage(&staging, AssetId::new(), &sha256, size, &source, None, None)
                    .unwrap_err();
            match error {
                crate::ProjectError::Io(error) => {
                    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                    if size == b"image bytes".len() as u64 {
                        assert_eq!(
                            error.to_string(),
                            format!("staged copy exceeds declared size {size}")
                        );
                    } else {
                        assert_eq!(
                            error.to_string(),
                            "staged asset bytes do not match the declared SHA-256"
                        );
                    }
                }
                error => panic!("{error:?}"),
            }
        }
    }

    #[test]
    fn payload_lease_refuses_a_short_copy() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let source = temporary.path().join("mesh.ply");
        std::fs::write(&source, b"ply bytes").unwrap();
        // The declared size disagrees with the actual file: refuse rather
        // than stage bytes that do not match the model's declared hash.
        let error = PayloadLease::stage(
            &staging,
            AssetId::new(),
            &sha256hex(b"ply bytes"),
            99,
            &source,
            None,
            None,
        )
        .unwrap_err();
        match error {
            crate::ProjectError::Io(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                assert_eq!(
                    error.to_string(),
                    format!("staged copy has 9 bytes, {} declared 99", source.display())
                );
            }
            error => panic!("{error:?}"),
        }
        assert!(
            std::fs::read_dir(&staging).unwrap().next().is_none(),
            "a refused stage leaves nothing behind"
        );
    }

    fn sha256hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
}
