//! UI-owned source document, independent of the rendering implementation.
use crate::{Directive, SceneParseError, lexer::Lexer, params::DirectiveReader};
use protocol::package::Dependency;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub struct Document {
    pub directives: Vec<Directive>,
    pub sources: BTreeMap<PathBuf, Dependency>,
}
#[derive(Clone, Copy)]
pub struct ReadOptions {
    pub include_depth: u32,
    pub expand_imports: bool,
    pub max_expanded_source_bytes: usize,
}
impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            include_depth: 64,
            expand_imports: true,
            max_expanded_source_bytes: 128 * 1024 * 1024,
        }
    }
}
pub fn read_document(path: &Path) -> Result<Document, SceneParseError> {
    read_document_with_options(path, ReadOptions::default())
}
pub fn read_document_with_options(
    path: &Path,
    options: ReadOptions,
) -> Result<Document, SceneParseError> {
    let mut document = Document {
        directives: Vec::new(),
        sources: BTreeMap::new(),
    };
    read(path, &mut Vec::new(), &mut document, options)?;
    Ok(document)
}
fn read(
    path: &Path,
    stack: &mut Vec<PathBuf>,
    document: &mut Document,
    options: ReadOptions,
) -> Result<(), SceneParseError> {
    let path = protocol::path::canonicalize(path)
        .map_err(|e| SceneParseError::unreadable(&path.display().to_string(), e.to_string()))?;
    let name = path
        .to_str()
        .ok_or_else(|| SceneParseError::unreadable("<path>", "source path is not UTF-8"))?;
    if stack.contains(&path) {
        return Err(SceneParseError::IncludeCycle {
            path: name.into(),
            line: 1,
            cycle: name.into(),
        });
    }
    if stack.len() > options.include_depth as usize {
        return Err(SceneParseError::IncludeDepth {
            path: name.into(),
            line: 1,
            limit: options.include_depth,
        });
    }
    let compressed_size = fs::metadata(&path)
        .map_err(|error| SceneParseError::unreadable(name, error.to_string()))?
        .len();
    if compressed_size > options.max_expanded_source_bytes as u64 {
        return Err(SceneParseError::ResourceExhausted {
            message: format!(
                "source {name} is larger than the {} byte preview source budget",
                options.max_expanded_source_bytes
            ),
        });
    }
    let raw = fs::read(&path).map_err(|e| SceneParseError::unreadable(name, e.to_string()))?;
    let hash = format!("{:x}", Sha256::digest(&raw));
    if document
        .sources
        .get(&path)
        .is_some_and(|previous| previous.sha256 != hash)
    {
        return Err(SceneParseError::unreadable(
            name,
            "source changed between reads",
        ));
    }
    document.sources.insert(
        path.clone(),
        Dependency {
            path: name.into(),
            role: "scene_source".into(),
            size: raw.len() as u64,
            sha256: hash,
        },
    );
    let source = if path.extension().is_some_and(|ext| ext == "gz") {
        let mut source = String::new();
        flate2::read::GzDecoder::new(raw.as_slice())
            .take(options.max_expanded_source_bytes as u64 + 1)
            .read_to_string(&mut source)
            .map_err(|e| SceneParseError::unreadable(name, e.to_string()))?;
        if source.len() > options.max_expanded_source_bytes {
            return Err(SceneParseError::ResourceExhausted {
                message: format!(
                    "expanded source {name} exceeds the {} byte preview source budget",
                    options.max_expanded_source_bytes
                ),
            });
        }
        source
    } else {
        if raw.len() > options.max_expanded_source_bytes {
            return Err(SceneParseError::ResourceExhausted {
                message: format!(
                    "source {name} exceeds the {} byte preview source budget",
                    options.max_expanded_source_bytes
                ),
            });
        }
        String::from_utf8(raw).map_err(|e| SceneParseError::unreadable(name, e.to_string()))?
    };
    let mut lexer = Lexer::new(name, &source);
    let mut reader = DirectiveReader::new(&mut lexer);
    stack.push(path.clone());
    while let Some(mut directive) = reader.read_directive()? {
        if directive.name == "Import" && !options.expand_imports {
            document.directives.push(directive);
            continue;
        }
        if matches!(directive.name.as_str(), "Include" | "Import") {
            // pbrt-v4 and r4 splice Include into the token stream, and the
            // project importer (pbrt-io/src/source.rs) reads it the same
            // way: parameters after the directive continue the included
            // file's last directive rather than belonging to Include.
            let mut continuation = std::mem::take(&mut directive.parameters);
            let requested = Path::new(directive.positional_text(0).unwrap_or(""));
            let resolved = if requested.is_absolute() {
                requested.to_path_buf()
            } else {
                stack
                    .iter()
                    .rev()
                    .map(|p| p.parent().unwrap().join(requested))
                    .find(|p| p.exists())
                    .unwrap_or_else(|| path.parent().unwrap().join(requested))
            };
            let before = document.directives.len();
            read(&resolved, stack, document, options)?;
            if !continuation.is_empty() && document.directives.len() > before {
                document
                    .directives
                    .last_mut()
                    .expect("the directive the continuation lands on")
                    .parameters
                    .append(&mut continuation);
            }
        } else {
            document.directives.push(directive);
        }
    }
    stack.pop();
    Ok(())
}
