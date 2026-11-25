//! The regenerable cache under `Library/Cache/`.
//!
//! Not project content: the cache is never journaled, never copied by Save
//! As and may be deleted at any time — `ensure_cache` recreates it. The
//! process holding the project lock writes it; every file and the index land
//! through a temporary file and rename. The index (`cache.json`) maps
//! `<item>/<key>` to its entry and carries the last-use stamp for eviction
//! past the size limit.
use crate::{Result, invalid};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{SystemTime, UNIX_EPOCH},
};

static ROOT_LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

fn root_lock(root: &Path) -> Arc<Mutex<()>> {
    let root = normalize_root(root);
    let mut locks = ROOT_LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&root).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(root, Arc::downgrade(&lock));
    lock
}

fn normalize_root(root: &Path) -> PathBuf {
    let absolute = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(root)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// Default size limit: 1 GiB.
pub const DEFAULT_CACHE_LIMIT: u64 = 1024 * 1024 * 1024;
const CACHE_FORMAT: &str = "pbrt-ui.cache/1";

/// A located cache entry. Readers get the absolute path and read it directly.
pub struct CacheHit {
    pub path: PathBuf,
    pub media_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    kind: String,
    media_type: String,
    size: u64,
    last_used: u128,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheIndex {
    format: String,
    entries: BTreeMap<String, CacheEntry>,
}

/// The cache directories; keeps a deleted `Library/` from breaking anything.
pub fn ensure_cache(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("Thumbnails"))?;
    fs::create_dir_all(root.join("Previews"))?;
    Ok(())
}

/// Path-safe cache identifiers: cores do not interpret keys, so the key must
/// stay a file name.
fn safe(component: &str) -> bool {
    // The dot components never name a file a cache write should touch.
    (component != "." && component != "..")
        && !component.is_empty()
        && component.len() <= 128
        && component.bytes().all(
            |byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-'),
        )
}

fn kind_directory(kind: &str) -> Option<&'static str> {
    match kind {
        "thumbnail" => Some("Thumbnails"),
        "preview" => Some("Previews"),
        _ => None,
    }
}

fn entry_key(item: &str, key: &str) -> String {
    format!("{item}/{key}")
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn index_path(root: &Path) -> PathBuf {
    root.join("cache.json")
}

/// A broken or unreadable index is not an error: the cache is regenerable.
fn read_index(root: &Path) -> CacheIndex {
    let Ok(bytes) = fs::read(index_path(root)) else {
        return CacheIndex::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn write_index(root: &Path, index: &CacheIndex) -> Result<()> {
    let value = CacheIndex {
        format: CACHE_FORMAT.to_owned(),
        entries: index.entries.clone(),
    };
    atomic_write(&index_path(root), &serde_json::to_vec(&value)?)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

/// Drop the least recently used entries past `limit`, keeping at least one.
/// Index content is untrusted: an entry whose components are not file names
/// or whose kind is unknown never turns into a file-system path — it is just
/// dropped, and its size still leaves the accounting.
fn evict(root: &Path, index: &mut CacheIndex, limit: u64) {
    let mut total: u128 = index.entries.values().map(|entry| entry.size as u128).sum();
    while total > u64::MAX.min(limit).max(1) as u128 && index.entries.len() > 1 {
        let victim = index
            .entries
            .iter()
            .min_by_key(|(key, entry)| (entry.last_used, (*key).clone()))
            .map(|(key, _)| key.clone());
        let Some(victim) = victim else {
            break;
        };
        let Some(entry) = index.entries.remove(&victim) else {
            break;
        };
        let Some(directory) = kind_directory(&entry.kind) else {
            // Unknown kind, corrupted index: nothing to delete on disk, but
            // its size must still leave the total.
            total = total.saturating_sub(entry.size as u128);
            continue;
        };
        let components = victim.split_once('/');
        let (item, key) = match components {
            Some((item, key)) => (item, key),
            None => {
                total = total.saturating_sub(entry.size as u128);
                continue;
            }
        };
        if safe(item) && safe(key) {
            // Only ever delete inside the cache directories; a doctored
            // index must not reach anywhere else.
            let _ = fs::remove_file(root.join(directory).join(item).join(key));
        }
        total = total.saturating_sub(entry.size as u128);
    }
}

/// Write one cache entry and return its path. `limit` bounds the total size
/// (the wire handler passes `DEFAULT_CACHE_LIMIT`; callers testing eviction
/// may pass smaller limits).
pub fn cache_put(
    root: &Path,
    item: &str,
    key: &str,
    kind: &str,
    media_type: &str,
    data: &[u8],
    limit: u64,
) -> Result<PathBuf> {
    let lock = root_lock(root);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !safe(item) || !safe(key) {
        return Err(invalid("cache item and key must be file names"));
    }
    let Some(directory) = kind_directory(kind) else {
        return Err(invalid("cache kind must be thumbnail or preview"));
    };
    ensure_cache(root)?;
    let mut index = read_index(root);
    let path = root.join(directory).join(item).join(key);
    fs::create_dir_all(path.parent().unwrap())?;
    // A hidden sibling name: two entries must never share a temporary file.
    let temporary = path.with_file_name(format!(".{key}.tmp"));
    fs::write(&temporary, data)?;
    fs::rename(&temporary, &path)?;
    index.entries.insert(
        entry_key(item, key),
        CacheEntry {
            kind: kind.to_owned(),
            media_type: media_type.to_owned(),
            size: data.len() as u64,
            last_used: now_nanos(),
        },
    );
    evict(root, &mut index, limit);
    write_index(root, &index)?;
    Ok(path)
}

/// Read one cache entry, refreshing its last-use stamp. Entries whose file
/// vanished heal to a miss; files the index does not know are not hits.
pub fn cache_get(root: &Path, item: &str, key: &str) -> Result<Option<CacheHit>> {
    let lock = root_lock(root);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !safe(item) || !safe(key) {
        return Err(invalid("cache item and key must be file names"));
    }
    let mut index = read_index(root);
    let Some(entry) = index.entries.get(&entry_key(item, key)).cloned() else {
        return Ok(None);
    };
    let Some(directory) = kind_directory(&entry.kind) else {
        index.entries.remove(&entry_key(item, key));
        write_index(root, &index)?;
        return Ok(None);
    };
    let path = root.join(directory).join(item).join(key);
    if !path.is_file() {
        index.entries.remove(&entry_key(item, key));
        write_index(root, &index)?;
        return Ok(None);
    }
    let media_type = entry.media_type.clone();
    index
        .entries
        .get_mut(&entry_key(item, key))
        .expect("entry just cloned")
        .last_used = now_nanos();
    write_index(root, &index)?;
    Ok(Some(CacheHit { path, media_type }))
}
