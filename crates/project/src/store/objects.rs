use super::*;
impl ObjectStore {
    /// Copy a reader into an immutable, content-addressed object. Existing
    /// objects are verified and reused; source paths never become references.
    pub fn put_object(&mut self, mut reader: impl Read) -> Result<ObjectRef> {
        regular_directory(&self.directory.join("objects"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(self.directory.join("objects"))?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .ok_or_else(|| invalid("object size overflow"))?;
            if size > MAX_OBJECT_BYTES {
                return Err(ProjectError::ResourceLimit(
                    "project object exceeds 4 GiB".into(),
                ));
            }
            temporary.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
        }
        let object = ObjectRef {
            sha256: format!("{:x}", hash.finalize()),
            size,
        };
        temporary.as_file().sync_all()?;
        let path = self.directory.join("objects").join(&object.sha256);
        match temporary.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                verified_file(&path, &object, MAX_OBJECT_BYTES)?;
            }
            Err(error) => return Err(error.error.into()),
        }
        crate::sync_path(self.directory.join("objects"))?;
        Ok(object)
    }

    pub fn open_object(&self, object: &ObjectRef) -> Result<File> {
        validate_hash(&object.sha256)?;
        regular_directory(&self.directory.join("objects"))?;
        verified_file(
            &self.directory.join("objects").join(&object.sha256),
            object,
            MAX_OBJECT_BYTES,
        )
    }

    /// Verified on-disk path of an immutable object, for same-filesystem
    /// bundle exports that hard-link instead of copying.
    pub fn object_path(&self, object: &ObjectRef) -> Result<std::path::PathBuf> {
        validate_hash(&object.sha256)?;
        regular_directory(&self.directory.join("objects"))?;
        let path = self.directory.join("objects").join(&object.sha256);
        verified_file(&path, object, MAX_OBJECT_BYTES)?;
        Ok(path)
    }

    pub(super) fn verify_objects(&self, document: &ProjectDocument) -> Result<()> {
        for object in document.object_refs()? {
            self.open_object(&object)?;
        }
        Ok(())
    }
}
