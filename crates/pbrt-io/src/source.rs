use crate::Result;
use crate::*;
use project::*;
use scene_parse::{Directive, lexer::Lexer, params::DirectiveReader};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::PathBuf,
};

pub(crate) struct Declaration {
    pub directive: Directive,
    pub origin: Origin,
    /// Search stacks by first parameter index: `(0, own)`, then one entry
    /// per Include continuation, so parameters read in a parent file
    /// resolve their files from the parent, as written (PR #333 review).
    pub search_dirs: Vec<(usize, Vec<PathBuf>)>,
}
pub(crate) struct Sources<'a, F> {
    pub store: &'a mut ObjectStore,
    pub options: &'a ImportOptions,
    pub cancelled: &'a F,
    pub record: ImportRecord,
    pub declarations: Vec<Declaration>,
    pub inputs: BTreeMap<PathBuf, ObjectRef>,
    paths: BTreeMap<PathBuf, SourceId>,
    total: u64,
    expanded_total: u64,
}
impl<'a, F: Fn() -> bool> Sources<'a, F> {
    pub fn new(store: &'a mut ObjectStore, options: &'a ImportOptions, cancelled: &'a F) -> Self {
        Self {
            store,
            options,
            cancelled,
            record: ImportRecord {
                id: ImportId::new(),
                importer_version: env!("CARGO_PKG_VERSION").into(),
                dialect: ImportDialect::PbrtR4,
                renderer_revision: Some(crate::R4_SEMANTIC_TARGET.into()),
                diagnostics: vec![],
                root: OccurrenceId::new(),
                sources: BTreeMap::new(),
                occurrences: BTreeMap::new(),
            },
            declarations: vec![],
            inputs: BTreeMap::new(),
            paths: BTreeMap::new(),
            total: 0,
            expanded_total: 0,
        }
    }
    pub(crate) fn copied_bytes(&self) -> u64 {
        self.record
            .sources
            .values()
            .filter_map(|source| source.expanded.as_ref())
            .fold(self.total, |total, object| {
                total.saturating_add(object.size)
            })
    }
    pub fn copy(&mut self, path: &Path) -> Result<ObjectRef> {
        check(self.cancelled)?;
        let path = protocol::path::canonicalize(path)?;
        let file = File::open(&path)?;
        let remaining = self.options.max_copied_bytes.saturating_sub(self.total);
        let mut reader = BudgetReader {
            reader: file,
            remaining,
            cancelled: self.cancelled,
        };
        let copied = self.store.put_object(&mut reader);
        check(self.cancelled)?;
        let object = copied?;
        self.total = self
            .total
            .checked_add(object.size)
            .ok_or_else(|| invalid("copy byte count overflow"))?;
        if self
            .inputs
            .get(&path)
            .is_some_and(|previous| previous != &object)
        {
            return Err(Error::SourceChanged(path.display().to_string()));
        }
        self.inputs.insert(path, object.clone());
        Ok(object)
    }
    pub fn read(
        &mut self,
        path: &Path,
        stack: &mut Vec<PathBuf>,
        parent: Option<(OccurrenceId, SourceSpan)>,
    ) -> Result<()> {
        check(self.cancelled)?;
        if stack.len() >= self.options.max_include_depth {
            return Err(invalid(format!(
                "includes nested more than {} deep (import limit)",
                self.options.max_include_depth
            )));
        }
        let path = protocol::path::canonicalize(path)?;
        if stack.contains(&path) {
            return Err(invalid("include cycle"));
        }
        if fs::metadata(&path)?.len() > self.options.max_source_bytes {
            return Err(invalid(format!(
                "source file larger than {} bytes (import limit)",
                self.options.max_source_bytes
            )));
        }
        let original = self.copy(&path)?;
        let bytes = self.store.open_object(&original)?;
        let mut source = String::new();
        if path.extension().is_some_and(|ext| ext == "gz") {
            flate2::read::GzDecoder::new(bytes)
                .take(self.options.max_source_bytes + 1)
                .read_to_string(&mut source)?;
        } else {
            bytes
                .take(self.options.max_source_bytes + 1)
                .read_to_string(&mut source)?;
        }
        if source.len() as u64 > self.options.max_source_bytes {
            return Err(invalid(format!(
                "expanded source larger than {} bytes (import limit)",
                self.options.max_source_bytes
            )));
        }
        self.expanded_total = self
            .expanded_total
            .checked_add(source.len() as u64)
            .ok_or_else(|| invalid("expanded byte count overflow"))?;
        if self.expanded_total > self.options.max_expanded_bytes {
            return Err(invalid(format!(
                "includes expand to more than {} bytes in total (import limit)",
                self.options.max_expanded_bytes
            )));
        }
        let source_id = if let Some(id) = self.paths.get(&path) {
            *id
        } else {
            let id = SourceId::new();
            let expanded = if path.extension().is_some_and(|ext| ext == "gz") {
                Some(self.store.put_object(source.as_bytes())?)
            } else {
                None
            };
            self.record.sources.insert(
                id,
                ImportedSource {
                    original_path: path
                        .to_str()
                        .ok_or_else(|| invalid("source path is not UTF-8"))?
                        .into(),
                    original,
                    expanded,
                },
            );
            self.paths.insert(path.clone(), id);
            id
        };
        let occurrence = if parent.is_none() {
            self.record.root
        } else {
            OccurrenceId::new()
        };
        let (parent, callsite) = parent.map_or((None, None), |(id, span)| (Some(id), Some(span)));
        self.record.occurrences.insert(
            occurrence,
            SourceOccurrence {
                source: source_id,
                parent,
                callsite,
            },
        );
        if self.record.occurrences.len() > self.options.max_nodes {
            return Err(invalid(format!(
                "more than {} source files (import limit)",
                self.options.max_nodes
            )));
        }
        stack.push(path.clone());
        let mut lexer = Lexer::new(path.to_str().unwrap(), &source);
        let mut reader =
            DirectiveReader::with_token_limit(&mut lexer, self.options.max_directive_tokens);
        loop {
            let mut directive = match reader.read_directive() {
                Ok(Some(directive)) => directive,
                Ok(None) => break,
                // Malformed input: skip to the next directive and keep reading.
                Err(scene_parse::SceneParseError::Syntax { message, .. }) => {
                    let skipped = reader.recover()?;
                    self.record.diagnostics.push(ImportDiagnostic::warning(
                        "syntax",
                        format!(
                            "skipped malformed input at {}:{}: {message}",
                            path.display(),
                            skipped.line
                        ),
                        Some(Origin {
                            import: self.record.id,
                            occurrence,
                            span: SourceSpan {
                                offset: skipped.offset,
                                length: skipped.length,
                                line: skipped.line,
                                column: skipped.column,
                            },
                        }),
                    ));
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            check(self.cancelled)?;
            let origin = Origin {
                import: self.record.id,
                occurrence,
                span: SourceSpan {
                    offset: directive.offset,
                    length: directive.length,
                    line: directive.line,
                    column: directive.column,
                },
            };
            if matches!(directive.name.as_str(), "Include" | "Import") {
                // pbrt-v4 and r4 splice Include into the token stream:
                // parameters after the directive continue the last
                // directive the included file produced (smoke-plume splits
                // a MakeNamedMedium's parameter list this way).
                let mut continuation = std::mem::take(&mut directive.parameters);
                let requested = directive.positional_text(0).unwrap_or("");
                let target = resolve(requested, stack).and_then(|target| {
                    if stack.contains(&target) {
                        Err(invalid("include cycle"))
                    } else {
                        Ok(target)
                    }
                });
                match target {
                    Ok(target) => {
                        let before = self.declarations.len();
                        self.read(&target, stack, Some((occurrence, origin.span.clone())))?;
                        if !continuation.is_empty() {
                            if self.declarations.len() == before {
                                self.record.diagnostics.push(ImportDiagnostic::warning(
                                    "unresolved_include",
                                    format!(
                                        "skipped {} \"{requested}\" at {}:{}: {}",
                                        directive.name,
                                        directive.path,
                                        directive.line,
                                        "Include parameters with no directive to continue"
                                    ),
                                    Some(origin),
                                ));
                            } else {
                                let last = self
                                    .declarations
                                    .last_mut()
                                    .expect("a declaration to extend");
                                last.search_dirs
                                    .push((last.directive.parameters.len(), stack.clone()));
                                last.directive.parameters.append(&mut continuation);
                            }
                        }
                    }
                    Err(Error::Unsupported(reason) | Error::Invalid(reason)) => {
                        self.record.diagnostics.push(ImportDiagnostic::warning(
                            "unresolved_include",
                            format!(
                                "skipped {} \"{requested}\" at {}:{}: {reason}",
                                directive.name, directive.path, directive.line
                            ),
                            Some(origin),
                        ));
                    }
                    Err(error) => return Err(error),
                }
            } else {
                if self.declarations.len() >= self.options.max_nodes {
                    return Err(invalid(format!(
                        "more than {} directives (import limit)",
                        self.options.max_nodes
                    )));
                }
                self.declarations.push(Declaration {
                    directive,
                    origin,
                    search_dirs: vec![(0, stack.clone())],
                });
            }
        }
        stack.pop();
        Ok(())
    }
    pub fn verify_inputs(&self) -> Result<()> {
        for (path, expected) in &self.inputs {
            check(self.cancelled)?;
            let mut file = File::open(path)?;
            let mut hash = Sha256::new();
            let mut size = 0u64;
            let mut buffer = [0; 65536];
            loop {
                check(self.cancelled)?;
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                size += count as u64;
                if size > expected.size {
                    return Err(Error::SourceChanged(path.display().to_string()));
                }
                hash.update(&buffer[..count]);
            }
            if size != expected.size || format!("{:x}", hash.finalize()) != expected.sha256 {
                return Err(Error::SourceChanged(path.display().to_string()));
            }
        }
        Ok(())
    }
}
pub(crate) fn resolve(requested: &str, stack: &[PathBuf]) -> Result<PathBuf> {
    if requested.is_empty() {
        return Err(invalid("empty dependency path"));
    }
    let requested = Path::new(requested);
    if requested.is_absolute() {
        return protocol::path::canonicalize(requested)
            .ok()
            .filter(|path| path.is_file())
            .ok_or_else(|| invalid(format!("unresolved dependency: {}", requested.display())));
    }
    for source in stack.iter().rev() {
        let candidate = source.parent().unwrap().join(requested);
        if candidate.is_file() {
            return Ok(protocol::path::canonicalize(candidate)?);
        }
    }
    Err(invalid(format!(
        "unresolved dependency: {}",
        requested.display()
    )))
}
struct BudgetReader<'a, R, F> {
    reader: R,
    remaining: u64,
    cancelled: &'a F,
}
impl<R: Read, F: Fn() -> bool> Read for BudgetReader<'_, R, F> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "import cancelled",
            ));
        }
        let limit = buffer.len().min(self.remaining.saturating_add(1) as usize);
        let count = self.reader.read(&mut buffer[..limit])?;
        if count as u64 > self.remaining {
            return Err(std::io::Error::other("import copy budget exceeded"));
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

#[cfg(test)]
mod copied_bytes_tests {
    use super::*;
    use std::io::Write;

    fn gzipped(directory: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
        let path = directory.join(name);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(text.as_bytes()).unwrap();
        std::fs::write(&path, encoder.finish().unwrap()).unwrap();
        path
    }
    fn plain(directory: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    fn sources(directory: &std::path::Path) -> (ObjectStore, ImportOptions, impl Fn() -> bool) {
        let store = ObjectStore::create(&directory.join("store")).unwrap();
        let cancelled = || false;
        (store, ImportOptions::default(), cancelled)
    }
    fn read(
        store: &mut ObjectStore,
        options: &ImportOptions,
        cancelled: &impl Fn() -> bool,
        path: &std::path::Path,
    ) -> u64 {
        let mut reader = Sources::new(store, options, cancelled);
        let mut stack = Vec::new();
        reader.read(path, &mut stack, None).unwrap();
        reader.copied_bytes()
    }
    /// One shared reader, as a worker uses during a single import job.
    fn reloaded(
        store: &mut ObjectStore,
        options: &ImportOptions,
        cancelled: &impl Fn() -> bool,
        path: &std::path::Path,
    ) -> (u64, u64) {
        let mut reader = Sources::new(store, options, cancelled);
        let mut stack = Vec::new();
        reader.read(path, &mut stack, None).unwrap();
        let once = reader.copied_bytes();
        reader.read(path, &mut stack, None).unwrap();
        (once, reader.copied_bytes())
    }

    /// The 8 GiB copy budget must count the recorded original
    /// objects and each gzip archive's expanded text as well, or a worker
    /// could pass a full spool of inflated archives through a budget that
    /// only saw the compressed originals.
    #[test]
    fn copied_bytes_count_originals_and_expanded_gzip_archives() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, options, cancelled) = sources(directory.path());
        let text = "WorldBegin\nWorldEnd\n";
        let gz = gzipped(directory.path(), "scene.pbrt.gz", text);
        let compressed = std::fs::metadata(&gz).unwrap().len();
        let expected = compressed + text.len() as u64;
        assert_eq!(read(&mut store, &options, &cancelled, &gz), expected);
        // A plain source counts once, byte for byte.
        let plain_text = plain(directory.path(), "plain.pbrt", text);
        assert_eq!(
            read(&mut store, &options, &cancelled, &plain_text),
            text.len() as u64
        );
    }

    /// Reading the same plain file twice within one job charges the budget
    /// twice, even
    /// though the spool content-addresses the bytes to a single object.
    #[test]
    fn repeated_includes_charge_the_copy_budget_again() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, options, cancelled) = sources(directory.path());
        let text = "WorldBegin\nWorldEnd\n";
        let source = plain(directory.path(), "scene.pbrt", text);
        let (once, twice) = reloaded(&mut store, &options, &cancelled, &source);
        assert_eq!(once, text.len() as u64);
        assert_eq!(twice, text.len() as u64 * 2);
    }

    /// Reading the same gzip archive twice within one job charges each
    /// physical copy of the original (`copy()` runs on every read), but the
    /// expansion is only materialized once per unique source path, so it
    /// is charged once: accounting follows what is actually stored.
    #[test]
    fn rereading_a_gzip_charges_copies_every_read_and_expansion_once() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, options, cancelled) = sources(directory.path());
        let text = "WorldBegin\nTranslate 1 0 0\nWorldEnd\n";
        let gz = gzipped(directory.path(), "scene.pbrt.gz", text);
        let compressed = std::fs::metadata(&gz).unwrap().len();
        let (once, twice) = reloaded(&mut store, &options, &cancelled, &gz);
        assert_eq!(once, compressed + text.len() as u64);
        assert_eq!(twice, once + compressed);
    }
}
