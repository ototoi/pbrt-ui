//! Artifact hashing, size validation and atomic exports.
use super::*;
pub(super) fn artifact(path: &Path, mime: &str, metadata: Value) -> Result<Artifact, ErrorPayload> {
    let mut file = fs::File::open(path).map_err(fail)?;
    let size = file.metadata().map_err(fail)?.len();
    if !file.metadata().map_err(fail)?.is_file() || size > 1024 * 1024 * 1024 {
        return Err(ErrorPayload::new(
            ErrorCode::ResourceExhausted,
            "artifact is not a regular file within the 1 GiB limit",
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(fail)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(Artifact {
        artifact_id: uuid::Uuid::new_v4(),
        path: path
            .to_str()
            .ok_or_else(|| fail("non UTF-8 artifact path"))?
            .into(),
        mime: mime.into(),
        size,
        sha256: format!("{:x}", digest.finalize()),
        metadata,
    })
}
pub(super) fn export(work: &ServiceTask) -> Result<Value, ErrorPayload> {
    match &work.work {
        Work::Export {
            artifact: source,
            path,
            overwrite,
        } => {
            let current = artifact(Path::new(&source.path), &source.mime, Value::Null)?;
            if current.sha256 != source.sha256 {
                return Err(ErrorPayload::new(
                    ErrorCode::SourceChanged,
                    "artifact changed",
                ));
            }
            let destination = Path::new(path);
            let parent = destination
                .parent()
                .ok_or_else(|| fail("export path has no parent"))?;
            let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(fail)?;
            std::io::copy(
                &mut fs::File::open(&source.path).map_err(fail)?,
                &mut temporary,
            )
            .map_err(fail)?;
            temporary.flush().map_err(fail)?;
            if *overwrite {
                temporary.persist(destination).map_err(fail)?;
            } else {
                temporary.persist_noclobber(destination).map_err(fail)?;
            }
            Ok(json!({"absolute_path":path,"artifact_id":source.artifact_id}))
        }
        _ => unreachable!("export dispatch"),
    }
}
