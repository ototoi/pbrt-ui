//! Atomic, hash-addressed geometry package output.
use crate::{error::SceneParseError, scene::PreviewGeometry};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};
pub type PreviewManifest = protocol::geometry::GeometryManifest;
pub type PreviewSummary = protocol::geometry::GeometryManifest;

/// Write the package atomically: each file lands through a `tmp` sibling
/// renamed into place, with `geometry.json` last so a partial package can
/// never publish. `durable` selects whether each file is
/// also fsynced before its rename: disposable preview slots pass `false`
/// and keep only the atomic publish; callers whose output users keep
/// pass `true`.
pub fn write_preview(
    preview: &PreviewGeometry,
    dir: &Path,
    durable: bool,
) -> Result<(), SceneParseError> {
    write_preview_with_store(preview, dir, durable, None)
}

/// Write a disposable package backed by a content-addressed blob store.
pub fn write_preview_with_store(
    preview: &PreviewGeometry,
    dir: &Path,
    durable: bool,
    store: Option<&Path>,
) -> Result<(), SceneParseError> {
    fs::create_dir_all(dir)
        .map_err(|e| SceneParseError::unreadable(&dir.display().to_string(), e.to_string()))?;
    if let Some(store) = store {
        fs::create_dir_all(store).map_err(|e| {
            SceneParseError::unreadable(&store.display().to_string(), e.to_string())
        })?;
    }
    for (name, file) in &preview.manifest.blobs {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                SceneParseError::unreadable(&parent.display().to_string(), e.to_string())
            })?
        }
        if let Some(store) = store {
            let stored = store.join(&file.sha256);
            if !stored.is_file() {
                let bytes = preview.blobs.get(name).ok_or_else(|| {
                    SceneParseError::unreadable(
                        name,
                        "blob bytes and content-store entry are missing",
                    )
                })?;
                let mut tmp = tempfile::Builder::new()
                    .prefix(".preview-blob-")
                    .tempfile_in(store)
                    .map_err(|e| {
                        SceneParseError::unreadable(&store.display().to_string(), e.to_string())
                    })?;
                tmp.write_all(bytes).map_err(|e| {
                    SceneParseError::unreadable(&tmp.path().display().to_string(), e.to_string())
                })?;
                match tmp.persist(&stored) {
                    Ok(_) => {}
                    Err(_error) if stored.is_file() => {}
                    Err(error) => {
                        return Err(SceneParseError::unreadable(
                            &stored.display().to_string(),
                            error.error.to_string(),
                        ));
                    }
                }
            }
            hard_link_or_copy(&stored, &path, |source, target| {
                fs::hard_link(source, target)
            })
            .map_err(|e| SceneParseError::unreadable(&path.display().to_string(), e.to_string()))?;
        } else {
            let bytes = preview
                .blobs
                .get(name)
                .ok_or_else(|| SceneParseError::unreadable(name, "blob bytes are missing"))?;
            let tmp = path.with_extension("tmp");
            let mut f = File::create(&tmp).map_err(|e| {
                SceneParseError::unreadable(&tmp.display().to_string(), e.to_string())
            })?;
            f.write_all(bytes).map_err(|e| {
                SceneParseError::unreadable(&tmp.display().to_string(), e.to_string())
            })?;
            if durable {
                f.sync_all().map_err(|e| {
                    SceneParseError::unreadable(&tmp.display().to_string(), e.to_string())
                })?;
            }
            fs::rename(&tmp, &path).map_err(|e| {
                SceneParseError::unreadable(&path.display().to_string(), e.to_string())
            })?;
        }
    }
    let bytes = serde_json::to_vec(&preview.manifest)
        .map_err(|e| SceneParseError::unreadable("geometry.json", e.to_string()))?;
    let tmp = dir.join("geometry.json.tmp");
    let mut f = File::create(&tmp)
        .map_err(|e| SceneParseError::unreadable("geometry.json", e.to_string()))?;
    f.write_all(&bytes)
        .map_err(|e| SceneParseError::unreadable("geometry.json", e.to_string()))?;
    if durable {
        f.sync_all()
            .map_err(|e| SceneParseError::unreadable("geometry.json", e.to_string()))?;
    }
    fs::rename(tmp, dir.join("geometry.json"))
        .map_err(|e| SceneParseError::unreadable("geometry.json", e.to_string()))?;
    let mut diagnostics = File::create(dir.join("diagnostics.jsonl"))
        .map_err(|e| SceneParseError::unreadable("diagnostics.jsonl", e.to_string()))?;
    for diagnostic in &preview.diagnostics {
        serde_json::to_writer(&mut diagnostics, diagnostic)
            .map_err(|e| SceneParseError::unreadable("diagnostics.jsonl", e.to_string()))?;
        diagnostics
            .write_all(b"\n")
            .map_err(|e| SceneParseError::unreadable("diagnostics.jsonl", e.to_string()))?;
    }
    if durable {
        diagnostics
            .sync_all()
            .map_err(|e| SceneParseError::unreadable("diagnostics.jsonl", e.to_string()))?;
    }
    Ok(())
}

fn hard_link_or_copy<F>(stored: &Path, target: &Path, hard_link: F) -> std::io::Result<()>
where
    F: FnOnce(&Path, &Path) -> std::io::Result<()>,
{
    hard_link(stored, target).or_else(|_| fs::copy(stored, target).map(|_| ()))
}

/// Read a package `write_preview` produced: the manifest, every blob listed
/// in it, and the diagnostics log. Capture/pick and the tests share this
/// whole-package reader; the desktop preview reads incrementally through
/// `read_preview_manifest` + `read_preview_blobs`.
pub fn read_preview_package(dir: &Path) -> Result<PreviewGeometry, SceneParseError> {
    let manifest = read_preview_manifest(dir)?;
    let blobs = read_preview_blobs_unchecked(dir, &manifest)?;
    let diagnostics = read_preview_diagnostics(dir)?;
    Ok(PreviewGeometry {
        manifest,
        blobs,
        diagnostics,
    })
}

/// The package's manifest alone: deciding which blobs an
/// install needs reads only `geometry.json`. The file is read whole then
/// parsed from memory — `from_reader` over a raw `File` walks it a byte
/// at a time, which alone cost more than every blob read combined at
/// villa scale.
pub fn read_preview_manifest(dir: &Path) -> Result<PreviewManifest, SceneParseError> {
    let bytes = fs::read(dir.join("geometry.json"))
        .map_err(|e| SceneParseError::unreadable("geometry.json", e.to_string()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| SceneParseError::unreadable("geometry.json", e.to_string()))
}

/// Only the named blobs of `manifest`. A name missing
/// from the manifest is skipped rather than failed: the caller decides
/// what is needed, and a stale name answers nothing, like today.
pub fn read_preview_blobs(
    dir: &Path,
    manifest: &PreviewManifest,
    names: &std::collections::BTreeSet<String>,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>, SceneParseError> {
    let mut blobs = std::collections::BTreeMap::new();
    for name in names {
        if !manifest.blobs.contains_key(name) {
            continue;
        }
        let bytes = fs::read(dir.join(name))
            .map_err(|e| SceneParseError::unreadable(name, e.to_string()))?;
        blobs.insert(name.clone(), bytes);
    }
    Ok(blobs)
}

/// Every blob the manifest lists, presence-checked the whole-package way.
fn read_preview_blobs_unchecked(
    dir: &Path,
    manifest: &PreviewManifest,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>, SceneParseError> {
    let mut blobs = std::collections::BTreeMap::new();
    for name in manifest.blobs.keys() {
        let bytes = fs::read(dir.join(name))
            .map_err(|e| SceneParseError::unreadable(name, e.to_string()))?;
        blobs.insert(name.clone(), bytes);
    }
    Ok(blobs)
}

/// Read the package's `diagnostics.jsonl` separately; the desktop install
/// reads blobs by name.
pub fn read_preview_diagnostics(dir: &Path) -> Result<Vec<crate::Diagnostic>, SceneParseError> {
    match fs::read_to_string(dir.join("diagnostics.jsonl")) {
        Ok(text) => text
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|e| SceneParseError::unreadable("diagnostics.jsonl", e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>(),
        // Empty diagnostics are legal output; treat a missing log as empty.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(SceneParseError::unreadable(
            "diagnostics.jsonl",
            error.to_string(),
        )),
    }
}

#[cfg(test)]
mod store_tests {
    use super::hard_link_or_copy;
    use std::fs;

    #[test]
    fn failed_hardlink_falls_back_to_copy() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("stored");
        let target = dir.path().join("bundle");
        fs::write(&source, b"blob").unwrap();
        hard_link_or_copy(&source, &target, |_, _| {
            Err(std::io::Error::other("simulated hardlink failure"))
        })
        .unwrap();
        assert_eq!(fs::read(target).unwrap(), b"blob");
    }
}
