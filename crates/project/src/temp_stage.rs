//! Private temp stages (`pbrt-ui-payloads-*`, `pbrt-ui-import-*`) carry
//! their creator's pid, and the creator holds one owner lock for its
//! lifetime. A killed or crashed process cannot remove its stages, so the
//! first stage of every process sweeps the stages whose owner lock is free.
//! Same pattern as the desktop's preview slots.
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const KINDS: [&str; 4] = [
    "pbrt-ui-payloads-",
    "pbrt-ui-import-",
    "pbrt-ui-viewer-",
    "pbrt-ui-jobs-",
];
const OWNER_PREFIX: &str = "pbrt-ui-stage-";

/// Create a stage of `kind` (one of `KINDS`) below `base`, readable by this user only:
/// staged payloads are hard links to project assets, and a world-readable
/// stage would expose them to other users.
pub(crate) fn tempdir_in(base: &Path, kind: &str) -> std::io::Result<tempfile::TempDir> {
    fs::create_dir_all(base)?;
    ensure_base(base)?;
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.prefix(&prefix(kind)).tempdir_in(base)
}

/// Create a swept, private temporary project root for a PBRT Viewer session.
pub fn viewer_tempdir_in(base: &Path) -> std::io::Result<tempfile::TempDir> {
    tempdir_in(base, KINDS[2])
}

/// Create a swept, private folder for one process's render and capture jobs.
pub fn jobs_tempdir_in(base: &Path) -> std::io::Result<tempfile::TempDir> {
    tempdir_in(base, KINDS[3])
}

/// Create directory components with private permissions where they are user
/// cache directories. Existing project directories retain their permissions.
pub(crate) fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Takes this process's owner lock before the first stage exists, and
/// starts the one-time sweep.
fn ensure_base(base: &Path) -> std::io::Result<()> {
    static OWNERS: OnceLock<Mutex<HashMap<PathBuf, Option<File>>>> = OnceLock::new();
    let pid = std::process::id();
    let owners = OWNERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut owners = owners.lock().expect("stage owner map poisoned");
    if !owners.contains_key(base) {
        // An owner file that is already there and free belongs to a dead
        // predecessor with this pid; the sweep skips its own pid, so its
        // stages go here, before this process creates any.
        let inherited = owner_path(base, pid).is_file();
        let owner = take_owner_lock(base, pid);
        if inherited
            && owner.is_some()
            && let Some(dirs) = stages_by_pid(base).remove(&pid)
        {
            remove_all(dirs);
        }
        owners.insert(base.to_path_buf(), owner);
        let base = base.to_path_buf();
        std::thread::spawn(move || sweep(&base, pid));
    }
    Ok(())
}

fn prefix(kind: &str) -> String {
    let pid = std::process::id();
    format!("{kind}{pid}-")
}

fn owner_path(base: &Path, pid: u32) -> PathBuf {
    base.join(format!("{OWNER_PREFIX}{pid}.owner"))
}

fn take_owner_lock(base: &Path, pid: u32) -> Option<File> {
    let path = owner_path(base, pid);
    // A symlinked lock would be followed into someone else's file.
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| !metadata.is_file())
    {
        return None;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .ok()?;
    // Without the lock this process's stages are never judged orphaned:
    // they only miss the sweep.
    file.try_lock().ok().map(|()| file)
}

/// `<kind><pid>-<random>` → pid; anything else (older random-only names,
/// unrelated entries) is never touched.
fn stage_pid(name: &str) -> Option<u32> {
    let rest = KINDS.iter().find_map(|kind| name.strip_prefix(kind))?;
    let (pid, random) = rest.split_once('-')?;
    (!random.is_empty() && !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
        .then(|| pid.parse().ok())
        .flatten()
}

fn owner_pid(name: &str) -> Option<u32> {
    let pid = name.strip_prefix(OWNER_PREFIX)?.strip_suffix(".owner")?;
    (!pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
        .then(|| pid.parse().ok())
        .flatten()
}

fn stages_by_pid(base: &Path) -> BTreeMap<u32, Vec<PathBuf>> {
    let mut stages: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(base) else {
        return stages;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(pid) = owner_pid(name) {
            stages.entry(pid).or_default();
        } else if let Some(pid) = stage_pid(name)
 // Directories only: a symlink is never followed or removed.
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            stages.entry(pid).or_default().push(entry.path());
        }
    }
    stages
}

/// Best effort: errors are skipped, and the next process retries.
fn sweep(base: &Path, own_pid: u32) {
    let stages = stages_by_pid(base);
    for (pid, dirs) in stages {
        if pid == own_pid {
            continue;
        }
        let path = owner_path(base, pid);
        // No owner file: a stage being created right now, or one whose
        // owner never got the lock. Locked: the owner is alive.
        if !path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_file())
        {
            continue;
        }
        let Ok(owner) = OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        if owner.try_lock().is_err() {
            continue;
        }
        // Holding the freed lock keeps a new process with the same pid out
        // until its predecessor's stages are gone. A stage that resists
        // removal keeps the owner file, so a later sweep retries it.
        if remove_all(dirs) {
            let _ = fs::remove_file(&path);
        }
    }
}

fn remove_all(dirs: Vec<PathBuf>) -> bool {
    dirs.into_iter()
        .fold(true, |all, dir| fs::remove_dir_all(dir).is_ok() && all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(base: &Path, kind: &str, pid: u32) -> PathBuf {
        let dir = base.join(format!("{kind}{pid}-AbC123"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("x.payload"), b"bytes").unwrap();
        dir
    }

    #[test]
    fn sweep_removes_stages_of_freed_owners_only() {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path();
        let own = 10;
        let (live, dead, no_owner) = (20, 30, 40);
        let own_stage = stage(base, KINDS[0], own);
        let live_stage = stage(base, KINDS[0], live);
        let dead_stages = [stage(base, KINDS[0], dead), stage(base, KINDS[1], dead)];
        let unknown = stage(base, KINDS[0], no_owner);
        let legacy = base.join("pbrt-ui-payloads-0a1616");
        fs::create_dir(&legacy).unwrap();
        let _live_owner = take_owner_lock(base, live).unwrap();
        // A dead owner leaves its file unlocked.
        File::create(owner_path(base, dead)).unwrap();
        File::create(owner_path(base, own)).unwrap();

        sweep(base, own);

        assert!(dead_stages.iter().all(|dir| !dir.exists()));
        assert!(!owner_path(base, dead).exists());
        for kept in [&own_stage, &live_stage, &unknown, &legacy] {
            assert!(kept.exists(), "{kept:?} must be kept");
        }
        assert!(owner_path(base, live).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_stage_that_resists_removal_keeps_its_owner_for_a_retry() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path();
        let dead = 30;
        let stuck = stage(base, KINDS[0], dead).join("held");
        fs::create_dir(&stuck).unwrap();
        fs::write(stuck.join("open"), b"bytes").unwrap();
        fs::set_permissions(&stuck, fs::Permissions::from_mode(0o500)).unwrap();
        File::create(owner_path(base, dead)).unwrap();

        sweep(base, 10);
        let kept = owner_path(base, dead).exists();
        fs::set_permissions(&stuck, fs::Permissions::from_mode(0o700)).unwrap();
        sweep(base, 10);

        assert!(kept, "the owner file must survive a failed removal");
        assert!(!stuck.exists() && !owner_path(base, dead).exists());
    }

    #[test]
    fn a_reused_pid_clears_its_predecessors_stages_first() {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path();
        let own = std::process::id();
        let old = stage(base, KINDS[3], own);
        File::create(owner_path(base, own)).unwrap();

        let new = tempdir_in(base, KINDS[3]).unwrap();

        assert!(
            !old.exists(),
            "the predecessor's stage goes before the first new one"
        );
        assert!(new.path().exists());
    }

    #[test]
    fn first_stage_sweeps_only_its_own_base() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let dead_pid = std::process::id().wrapping_add(100_000);
        let first_orphan = stage(first.path(), KINDS[0], dead_pid);
        let second_orphan = stage(second.path(), KINDS[0], dead_pid);
        File::create(owner_path(first.path(), dead_pid)).unwrap();
        File::create(owner_path(second.path(), dead_pid)).unwrap();

        let _stage = tempdir_in(first.path(), KINDS[0]).unwrap();
        for _ in 0..100 {
            if !first_orphan.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert!(!first_orphan.exists());
        assert!(second_orphan.exists());
    }

    #[cfg(unix)]
    #[test]
    fn stages_are_private_to_their_user() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let stage = tempdir_in(temporary.path(), KINDS[0]).unwrap();
        let mode = fs::metadata(stage.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn only_pid_named_stages_parse() {
        assert_eq!(stage_pid("pbrt-ui-payloads-123-Xy9z0a"), Some(123));
        assert_eq!(stage_pid("pbrt-ui-import-7-Xy9z0a"), Some(7));
        assert_eq!(stage_pid("pbrt-ui-viewer-7-Xy9z0a"), Some(7));
        assert_eq!(stage_pid("pbrt-ui-jobs-7-Xy9z0a"), Some(7));
        assert_eq!(stage_pid("pbrt-ui-payloads-0a1616"), None);
        assert_eq!(stage_pid("pbrt-ui-payloads-123456"), None);
        assert_eq!(stage_pid("pbrt-ui-preview-1-2-3"), None);
        assert_eq!(owner_pid("pbrt-ui-stage-55.owner"), Some(55));
        assert_eq!(owner_pid("pbrt-ui-stage-x.owner"), None);
    }
}
