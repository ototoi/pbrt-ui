//! One owner lock per desktop process protects the preview
//! slots it creates, and the startup sweep that clears slots orphaned
//! by desktops that never exited. Exit cleanup
//! is unchanged.
//!
//! One lock per process, not per slot: the core replaces slot contents
//! on publish, so a file inside a slot would not survive it.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// `state_sync` names slot directories
/// `pbrt-ui-preview-<pid>-<project id>-<activation>`; the owner lock
/// file is `pbrt-ui-preview-<pid>.owner` beside them.
const SLOT_PREFIX: &str = "pbrt-ui-preview-";
const STORE_PREFIX: &str = "pbrt-ui-preview-store-";

/// The owner lock of one desktop process. The flock belongs to the open
/// file description, so holding the `File` for the process lifetime (in
/// the display state) holds the lock.
pub(super) struct PreviewSlotOwner {
    _file: File,
}

/// Initialize each preview base once. Locks remain held for the process
/// lifetime, including after a project is closed while jobs still hold leases.
pub(super) fn initialize_base(base: &Path, private_cache: bool) -> bool {
    static BASES: OnceLock<Mutex<std::collections::HashMap<PathBuf, Option<PreviewSlotOwner>>>> =
        OnceLock::new();
    let bases = BASES.get_or_init(Default::default);
    let mut bases = bases.lock().expect("preview base map poisoned");
    if bases.contains_key(base) {
        return true;
    }
    if fs::create_dir_all(base).is_err() {
        eprintln!("pbrt-ui: cannot create preview base; preview sweep is unavailable");
        return false;
    }
    if private_cache {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for directory in [base.parent(), Some(base)].into_iter().flatten() {
                if fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).is_err() {
                    eprintln!("pbrt-ui: cannot make preview cache directories private");
                    return false;
                }
            }
        }
    }
    let pid = std::process::id();
    let (owner, own_pid_stale) = take_owner_lock(base, pid);
    if own_pid_stale {
        remove_own_slots(base, pid);
    }
    bases.insert(base.to_path_buf(), owner);
    let base = base.to_path_buf();
    std::thread::spawn(move || sweep_orphaned_slots(&base, pid));
    true
}

pub(super) fn store_path_for_slot(slot: &Path) -> PathBuf {
    let name = slot
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let suffix = name.strip_prefix(SLOT_PREFIX).unwrap_or(name);
    slot.with_file_name(format!("{STORE_PREFIX}{suffix}"))
}

pub(super) fn remove_slot_and_store(slot: &Path) {
    let _ = fs::remove_dir_all(slot);
    let _ = fs::remove_dir_all(store_path_for_slot(slot));
}

/// The owning pid of a slot name `state_sync` produces —
/// `<pid>-<project id: ascii alphanumerics, possibly empty>-<activation>` —
/// or None for anything else in the temp dir, so the sweep never
/// touches unrelated entries (`pbrt-ui-mesh-*`, exports, user files).
/// The `.owner` lock files never parse as slots.
fn slot_pid(name: &str) -> Option<u32> {
    let rest = name
        .strip_prefix(STORE_PREFIX)
        .or_else(|| name.strip_prefix(SLOT_PREFIX))?;
    let (pid, rest) = rest.split_once('-')?;
    let (project, activation) = rest.split_once('-')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(pid) || !digits(activation) || !project.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    pid.parse().ok()
}

/// Take this process's owner lock. `None`: the lock could not be
/// taken, and this desktop runs without one; a future desktop's sweep
/// then judges its slots by the legacy pid check.
///
/// The flag reports that the pid's pre-existing owner file was free to
/// take: a dead predecessor left it, so its leftover slots under this
/// pid are orphans too.
fn owner_pid(name: &str) -> Option<u32> {
    let pid = name.strip_prefix(SLOT_PREFIX)?.strip_suffix(".owner")?;
    (!pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
        .then(|| pid.parse().ok())
        .flatten()
}

pub(super) fn take_owner_lock(base: &Path, pid: u32) -> (Option<PreviewSlotOwner>, bool) {
    let owner_path = base.join(format!("{SLOT_PREFIX}{pid}.owner"));
    // Refuse anything but a regular file, like `project/src/store/files.rs`:
    // a symlinked lock would be followed into someone else's file.
    let preexisting = owner_path.symlink_metadata();
    if preexisting
        .as_ref()
        .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
    {
        eprintln!("pbrt-ui: preview owner lock file is not a regular file; continuing without it");
        return (None, false);
    }
    // Same std lock as the project store: flock lives on the open file
    // description, so keeping the handle open holds the lock.
    let file = match OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&owner_path)
    {
        Ok(file) => file,
        Err(_) => {
            eprintln!("pbrt-ui: cannot take the preview owner lock; continuing without it");
            return (None, false);
        }
    };
    match file.try_lock() {
        Ok(()) => (Some(PreviewSlotOwner { _file: file }), preexisting.is_ok()),
        Err(_) => {
            eprintln!(
                "pbrt-ui: another holder keeps the preview owner lock; continuing without it"
            );
            (None, false)
        }
    }
}

/// Remove a dead same-pid predecessor's slots. Synchronous, before the
/// first preview request: the async sweep could otherwise delete a slot
/// this process has just published under the same pid.
pub(super) fn remove_own_slots(base: &Path, own_pid: u32) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let is_own = entry.file_name().to_str().and_then(slot_pid) == Some(own_pid);
        if is_own && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let path = entry.path();
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(STORE_PREFIX)
            {
                let _ = fs::remove_dir_all(path);
            } else {
                remove_slot_and_store(&path);
            }
        }
    }
}

/// One-shot best-effort sweep of the temp dir at startup: remove the
/// slot directories of desktops that are gone, and the freed `.owner`
/// lock files with them. Nothing but `state_sync`'s slot names is ever
/// removed, and errors never break startup: the next desktop's sweep
/// retries what this one leaves behind.
pub(super) fn sweep_orphaned_slots(base: &Path, own_pid: u32) {
    let Ok(entries) = fs::read_dir(base) else {
        eprintln!("pbrt-ui: preview slot sweep cannot list the temp dir (best effort)");
        return;
    };
    // Group slots by owning pid — one verdict covers a desktop's whole
    // set — and only existing directories are candidates: symlinks are
    // never followed or removed.
    let mut slots: BTreeMap<u32, Vec<PathBuf>> = BTreeMap::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        // A cleanly exited desktop leaves only its `.owner`: visit that pid too.
        if let Some(pid) = owner_pid(name) {
            slots.entry(pid).or_default();
            continue;
        }
        let Some(pid) = slot_pid(name) else { continue };
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            slots.entry(pid).or_default().push(entry.path());
        }
    }
    let mut failures = 0u32;
    for (pid, dirs) in &slots {
        let mut remove_owner = false;
        let orphaned = if *pid == own_pid {
            // Ours: a stale same-pid predecessor's slots were already
            // removed by `remove_own_slots`, before any publication.
            false
        } else {
            let owner_path = base.join(format!("{SLOT_PREFIX}{pid}.owner"));
            match owner_path.symlink_metadata() {
                // Legacy slots from desktops predating the owner lock.
                Err(_) => legacy_pid_is_dead(*pid),
                Ok(metadata) if metadata.is_file() => {
                    // Acquirable means the creator is gone (a lock dies
                    // with its process); locked means left alone.
                    remove_owner = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&owner_path)
                        .is_ok_and(|file| file.try_lock().is_ok());
                    remove_owner
                }
                Ok(_) => false, // not a regular file: leave the pid alone
            }
        };
        if !orphaned {
            continue;
        }
        for dir in dirs {
            failures += u32::from(fs::remove_dir_all(dir).is_err());
        }
        if remove_owner {
            failures +=
                u32::from(fs::remove_file(base.join(format!("{SLOT_PREFIX}{pid}.owner"))).is_err());
        }
    }
    // Best effort, logged once: per-entry errors would only spam.
    if failures > 0 {
        eprintln!("pbrt-ui: preview slot sweep left {failures} entries behind (best effort)");
    }
}

/// Legacy slots (from desktops predating the owner lock) have no lock
/// file; only pid liveness can speak for them.
#[cfg(target_os = "linux")]
fn legacy_pid_is_dead(pid: u32) -> bool {
    // A reused pid runs another program: compare
    // with our own process name instead of trusting that /proc/<pid> exists.
    let comm = |pid: &str| fs::read(Path::new("/proc").join(pid).join("comm")).ok();
    comm(&pid.to_string()).is_none_or(|name| Some(name) != comm("self"))
}

// shortcut: without /proc there is no liveness check precise enough to
// bet a deletion on, so legacy slots stay for their desktop to clear.
#[cfg(not(target_os = "linux"))]
fn legacy_pid_is_dead(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// One slot directory in state_sync's naming, with a file inside so
    /// removals are observable.
    fn slot(base: &Path, pid: u32, project: &str, activation: u64) -> PathBuf {
        let dir = base.join(format!("{SLOT_PREFIX}{pid}-{project}-{activation}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("geometry.json"), "{}").unwrap();
        dir
    }

    fn store(base: &Path, pid: u32, project: &str, activation: u64) -> PathBuf {
        let dir = base.join(format!("{STORE_PREFIX}{pid}-{project}-{activation}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("blob"), "content").unwrap();
        dir
    }

    #[test]
    fn slot_pid_accepts_strict_slot_and_store_names() {
        assert_eq!(slot_pid("pbrt-ui-preview-42-project9-7"), Some(42));
        assert_eq!(slot_pid("pbrt-ui-preview-store-42-project9-7"), Some(42));
        for name in [
            "pbrt-ui-preview-store-42-project9-7-extra",
            "pbrt-ui-preview-store-42-project9-nope",
            "pbrt-ui-preview-store-x-project9-7",
            "pbrt-ui-preview-42-project-7-store",
        ] {
            assert_eq!(slot_pid(name), None, "{name} must not be swept");
        }
    }

    /// The owner lock file `pbrt-ui-preview-<pid>.owner`.
    fn owner_path(base: &Path, pid: u32) -> PathBuf {
        base.join(format!("{SLOT_PREFIX}{pid}.owner"))
    }

    /// Stand in for a live desktop: create and lock the owner file.
    fn owner_lock_file(base: &Path, pid: u32) -> File {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(owner_path(base, pid))
            .unwrap();
        file.try_lock().unwrap();
        file
    }

    /// The five verdicts of the sweep in one temp dir: a freed owner's
    /// slots and lock file go, a locked owner's stay, our own live
    /// slots stay, and unrelated entries are never touched.
    #[test]
    fn sweep_removes_freed_owners_and_keeps_live_ones_and_unrelated_entries() {
        let base = TempDir::new().unwrap();
        // Gone: lock file present but acquirable.
        let gone_a = slot(base.path(), 222_222, "a", 0);
        let gone_b = slot(base.path(), 222_222, "b2", 71);
        let gone_store = store(base.path(), 222_222, "a", 0);
        fs::write(owner_path(base.path(), 222_222), "").unwrap();
        // Gone after a clean exit: only the freed lock file is left.
        fs::write(owner_path(base.path(), 333_333), "").unwrap();
        // Alive: a locked owner.
        let holder = owner_lock_file(base.path(), 111_111);
        let kept_locked = slot(base.path(), 111_111, "a", 1);
        let kept_locked_store = store(base.path(), 111_111, "a", 1);
        // Alive: our own pid.
        let kept_own = slot(base.path(), std::process::id(), "pj", 4);
        let kept_own_store = store(base.path(), std::process::id(), "pj", 4);
        // Unrelated temp dir entries the sweep must not touch.
        let mesh = base.path().join("pbrt-ui-mesh-0123456789abcdef");
        fs::create_dir_all(&mesh).unwrap();
        let user_file = base.path().join("notes.txt");
        fs::write(&user_file, "x").unwrap();
        let non_slot_prev = base.path().join("pbrt-ui-preview-abc-proj-0");
        fs::create_dir_all(&non_slot_prev).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/nowhere", base.path().join("pbrt-ui-preview-222222-prj-0"))
            .unwrap();

        sweep_orphaned_slots(base.path(), std::process::id());

        assert!(!gone_a.exists(), "freed owner: its slots are removed");
        assert!(!gone_b.exists(), "freed owner: every slot of that pid goes");
        assert!(!gone_store.exists(), "freed owner: its stores are removed");
        assert!(
            !owner_path(base.path(), 222_222).exists(),
            "freed owner: lock file removed with the slots"
        );
        assert!(
            !owner_path(base.path(), 333_333).exists(),
            "a freed lock file with no slots is removed too"
        );
        assert!(kept_locked.is_dir(), "locked owner keeps its slots");
        assert!(kept_locked_store.is_dir(), "locked owner keeps its stores");
        assert!(
            owner_path(base.path(), 111_111).is_file(),
            "locked owner keeps its lock file"
        );
        assert!(kept_own.is_dir(), "our own live slots stay");
        assert!(kept_own_store.is_dir(), "our own live stores stay");
        assert!(
            mesh.is_dir() && user_file.is_file() && non_slot_prev.is_dir(),
            "unrelated names keep untouched"
        );
        #[cfg(unix)]
        assert!(
            base.path()
                .join("pbrt-ui-preview-222222-prj-0")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink(),
            "symlinked slot names are never followed or removed"
        );
        drop(holder);
    }

    #[test]
    fn removing_a_slot_removes_its_store() {
        let base = TempDir::new().unwrap();
        let slot = slot(base.path(), 8181, "project", 3);
        let store = store(base.path(), 8181, "project", 3);
        remove_slot_and_store(&slot);
        assert!(!slot.exists());
        assert!(!store.exists());
    }

    /// Legacy slots have no lock file; only the pid decides.
    #[cfg(target_os = "linux")]
    #[test]
    fn sweep_removes_legacy_slots_of_dead_pids() {
        let base = TempDir::new().unwrap();
        // Beyond any assignable Linux pid, so no live process holds it.
        let dead = slot(base.path(), 4_000_000_000, "pj", 0);
        assert!(legacy_pid_is_dead(4_000_000_000));
        assert!(!legacy_pid_is_dead(std::process::id()));
        assert!(
            legacy_pid_is_dead(1),
            "a pid running another program counts as gone"
        );

        sweep_orphaned_slots(base.path(), std::process::id());

        assert!(!dead.exists(), "legacy slot of a dead pid is removed");
    }

    /// Our own pid's slots are swept only when its pre-existing owner
    /// lock was stale when the startup took it.
    #[test]
    fn sweep_removes_own_pid_slots_only_when_its_owner_was_stale() {
        let base = TempDir::new().unwrap();
        let own = slot(base.path(), std::process::id(), "pj", 0);
        sweep_orphaned_slots(base.path(), std::process::id());
        assert!(own.is_dir(), "the async sweep never touches our pid");
        let other = slot(base.path(), 111_111, "pj", 0);
        remove_own_slots(base.path(), std::process::id());
        assert!(!own.exists(), "a stale same-pid owner orphans its slots");
        assert!(other.is_dir(), "only our pid's slots go");
    }

    #[test]
    fn owner_lock_is_taken_and_reports_free_predecessors_as_stale() {
        let base = TempDir::new().unwrap();
        let (guard, stale) = take_owner_lock(base.path(), std::process::id());
        assert!(guard.is_some(), "fresh owner file is taken");
        assert!(!stale, "nothing existed before");
        drop(guard); // Lock released; the file stays behind: the crash shape.
        let (guard, stale) = take_owner_lock(base.path(), std::process::id());
        assert!(guard.is_some());
        assert!(stale, "free pre-existing owner file: predecessor gone");
        let (busy, stale) = take_owner_lock(base.path(), std::process::id());
        assert!(
            busy.is_none(),
            "busy owner file leaves the newer desktop without a guard"
        );
        assert!(!stale);
        drop(guard);
    }

    #[test]
    fn each_preview_base_is_swept_only_when_first_used() {
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        let first_orphan = slot(first.path(), 4_000_000_000, "first", 1);
        let second_orphan = slot(second.path(), 4_000_000_000, "second", 1);
        fs::write(owner_path(first.path(), 4_000_000_000), "").unwrap();
        fs::write(owner_path(second.path(), 4_000_000_000), "").unwrap();

        assert!(initialize_base(first.path(), false));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while first_orphan.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!first_orphan.exists(), "the first used base is swept");
        assert!(second_orphan.exists(), "an unused base is not swept");

        assert!(initialize_base(second.path(), false));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while second_orphan.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            !second_orphan.exists(),
            "the second base is swept on first use"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cache_preview_directories_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let cache = TempDir::new().unwrap();
        let base = cache.path().join("Library/Preview");
        assert!(initialize_base(&base, true));
        for directory in [base.parent().unwrap(), base.as_path()] {
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
}
