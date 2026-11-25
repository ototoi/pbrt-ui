use super::*;

pub(super) fn validate_resource_cycles(doc: &ProjectDocument) -> Result<()> {
    let mut done = BTreeSet::new();
    for root in doc.resources.keys() {
        let mut active = BTreeSet::new();
        let mut stack = vec![(*root, false)];
        while let Some((id, exit)) = stack.pop() {
            if exit {
                active.remove(&id);
                done.insert(id);
                continue;
            }
            if done.contains(&id) {
                continue;
            }
            if !active.insert(id) {
                return Err(crate::ProjectError::ResourceCycle);
            }
            stack.push((id, true));
            for p in &doc.resources[&id].parameters {
                if let ParameterValues::Resources { ids, .. } = &p.values {
                    stack.extend(ids.iter().map(|id| (*id, false)));
                }
            }
        }
    }
    Ok(())
}
