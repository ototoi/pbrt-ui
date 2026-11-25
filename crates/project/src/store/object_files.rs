use super::*;

pub(super) fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("invalid object SHA-256"));
    }
    Ok(())
}
pub(super) fn verified_file(path: &Path, expected: &ObjectRef, limit: u64) -> Result<File> {
    validate_hash(&expected.sha256)?;
    if expected.size > limit {
        return Err(ProjectError::ResourceLimit("project file size".into()));
    }
    let mut file = regular_file(path)?;
    if file.metadata()?.len() != expected.size {
        return Err(invalid("project file size mismatch"));
    }
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > expected.size {
            return Err(invalid("project file changed while reading"));
        }
        hash.update(&buffer[..count]);
    }
    if size != expected.size || format!("{:x}", hash.finalize()) != expected.sha256 {
        return Err(invalid("project file digest mismatch"));
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}
