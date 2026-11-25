use super::*;

pub(super) fn lock(directory: &Path) -> Result<File> {
    let path = directory.join(".project.lock");
    if path
        .symlink_metadata()
        .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
    {
        return Err(invalid("project lock is not a regular file"));
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    try_lock_briefly(&file)?;
    Ok(file)
}

/// Take the project's exclusive lock, waiting briefly when it is busy.
///
/// The lock is a flock, which belongs to the open file description. A child
/// process forked by any thread while this process held the lock (renderer,
/// capture daemon) shares that description until it execs, so a reopen right
/// after a close or Save As can see `WouldBlock` for a moment. A project
/// another process really has open stays locked past the wait.
pub(super) fn try_lock_briefly(file: &File) -> Result<()> {
    const ATTEMPTS: u32 = 25;
    for attempt in 0..ATTEMPTS {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(fs::TryLockError::WouldBlock) if attempt + 1 < ATTEMPTS => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(fs::TryLockError::WouldBlock) => return Err(ProjectError::Locked),
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    Err(ProjectError::Locked)
}
pub(super) fn regular_directory(path: &Path) -> Result<()> {
    let metadata = path.symlink_metadata()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid("project directory is not a regular directory"));
    }
    Ok(())
}
pub(super) fn regular_file(path: &Path) -> Result<File> {
    let metadata = path.symlink_metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("project object is not a regular file"));
    }
    Ok(File::open(path)?)
}
pub(super) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = regular_file(path)?;
    if file.metadata()?.len() > limit {
        return Err(ProjectError::ResourceLimit("project metadata size".into()));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(ProjectError::ResourceLimit("project metadata size".into()));
    }
    Ok(bytes)
}
