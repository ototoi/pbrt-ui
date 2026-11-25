use super::*;

fn span(span: &SourceSpan, size: u64) -> Result<()> {
    if span.line == 0
        || span.column == 0
        || span
            .offset
            .checked_add(span.length)
            .is_none_or(|end| end > size)
    {
        return Err(invalid("source span exceeds its archived source"));
    }
    Ok(())
}
pub(super) fn validate_origin(doc: &ProjectDocument, origin: &Origin) -> Result<()> {
    let import = doc
        .imports
        .get(&origin.import)
        .ok_or_else(|| invalid("missing origin import"))?;
    let occurrence = import
        .occurrences
        .get(&origin.occurrence)
        .ok_or_else(|| invalid("missing source occurrence"))?;
    let source = import
        .sources
        .get(&occurrence.source)
        .ok_or_else(|| invalid("missing occurrence source"))?;
    span(
        &origin.span,
        source.expanded.as_ref().unwrap_or(&source.original).size,
    )
}
pub(super) fn validate_imports(doc: &ProjectDocument) -> Result<()> {
    for (id, import) in &doc.imports {
        if id != &import.id || !import.occurrences.contains_key(&import.root) {
            return Err(invalid("invalid import id or root"));
        }
        if import.importer_version.trim().is_empty() {
            return Err(invalid("missing importer version"));
        }
        for diagnostic in &import.diagnostics {
            if diagnostic.code.is_empty() || diagnostic.message.is_empty() {
                return Err(invalid("empty import diagnostic"));
            }
            if let Some(origin) = &diagnostic.origin {
                validate_origin(doc, origin)?;
            }
        }
        let mut children: BTreeMap<OccurrenceId, Vec<OccurrenceId>> = BTreeMap::new();
        for (id, occurrence) in &import.occurrences {
            if !import.sources.contains_key(&occurrence.source) {
                return Err(invalid("missing occurrence source"));
            }
            if *id == import.root {
                if occurrence.parent.is_some() || occurrence.callsite.is_some() {
                    return Err(invalid("import root has a parent/callsite"));
                }
            } else {
                let parent = occurrence
                    .parent
                    .and_then(|id| import.occurrences.get(&id))
                    .ok_or_else(|| invalid("missing source occurrence parent"))?;
                let source = import
                    .sources
                    .get(&parent.source)
                    .ok_or_else(|| invalid("missing parent source"))?;
                span(
                    occurrence
                        .callsite
                        .as_ref()
                        .ok_or_else(|| invalid("missing include/import callsite"))?,
                    source.expanded.as_ref().unwrap_or(&source.original).size,
                )?;
                children
                    .entry(occurrence.parent.unwrap())
                    .or_default()
                    .push(*id);
            }
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![(import.root, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            if depth > MAX_DEPTH || !seen.insert(id) {
                return Err(invalid("cyclic or excessively deep source occurrences"));
            }
            if let Some(children) = children.get(&id) {
                stack.extend(children.iter().map(|id| (*id, depth + 1)));
            }
        }
        if seen.len() != import.occurrences.len() {
            return Err(invalid("unreachable source occurrence"));
        }
    }
    Ok(())
}
