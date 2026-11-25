use super::*;

/// Hash a document the way format 1's manifest recorded it. Only used while
/// reading a legacy generation.
pub(super) fn fingerprint(value: &impl Serialize) -> Result<String> {
    let mut writer = HashWriter {
        hash: Sha256::new(),
        size: 0,
        exceeded: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        return Err(ProjectError::ResourceLimit("project metadata size".into()));
    }
    result?;
    Ok(format!("{:x}", writer.hash.finalize()))
}

struct HashWriter {
    hash: Sha256,
    size: u64,
    exceeded: bool,
}
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.size += bytes.len() as u64;
        if self.size > MAX_METADATA_BYTES {
            self.exceeded = true;
            return Err(std::io::Error::other("project metadata exceeds size limit"));
        }
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
