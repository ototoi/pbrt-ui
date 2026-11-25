use super::*;

/// Every content-addressed object a store must be able to open for this
/// document: asset bodies and the archived import sources.
pub(crate) fn objects(doc: &ProjectDocument) -> Result<Vec<ObjectRef>> {
    let mut refs = Vec::new();
    refs.extend(doc.assets.values().map(Asset::object));
    for import in doc.imports.values() {
        for source in import.sources.values() {
            refs.push(source.original.clone());
            refs.extend(source.expanded.clone());
        }
    }
    let mut unique = BTreeMap::new();
    for object in refs {
        if object.sha256.len() != 64
            || !object
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("invalid object SHA-256"));
        }
        if unique
            .insert(object.sha256.clone(), object.size)
            .is_some_and(|size| size != object.size)
        {
            return Err(invalid("conflicting object sizes for one hash"));
        }
    }
    Ok(unique
        .into_iter()
        .map(|(sha256, size)| ObjectRef { sha256, size })
        .collect())
}
