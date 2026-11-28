use crate::{
    ItemId, Node, NodeId, NodeKind, Parameter, ParameterValues, ProjectDocument, ResourceId,
    SceneId, StateOperation,
};
use std::collections::BTreeMap;

/// The route through which a reference occurs, such as a parameter's
/// name, or the binding's side.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReferenceVia {
    Material,
    MediumInside,
    MediumOutside,
    Instance,
    Parameter(String),
}

impl ReferenceVia {
    /// The wire's `via`: strings — `material`,
    /// `medium inside`, `medium outside`, `instance`, `parameter:NAME`.
    pub fn wire(&self) -> String {
        match self {
            Self::Material => "material".into(),
            Self::MediumInside => "medium inside".into(),
            Self::MediumOutside => "medium outside".into(),
            Self::Instance => "instance".into(),
            Self::Parameter(name) => format!("parameter:{name}"),
        }
    }
}

/// A single node or resource that refers to a project item.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReferenceLocation {
    pub scene: Option<SceneId>,
    pub node: Option<NodeId>,
    pub resource: Option<ResourceId>,
    pub via: ReferenceVia,
}

/// Project-wide reverse references, shared by Contents and deletion checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceIndex {
    references: BTreeMap<ItemId, Vec<ReferenceLocation>>,
    /// Owner resource of every node inside an ObjectDefinition scope: the
    /// definition's resource a "definition_node" reference reports.
    definition_owners: BTreeMap<NodeId, ResourceId>,
    /// Byte length a parameter array requires of its asset.
    array_bytes: BTreeMap<crate::AssetId, u64>,
}

impl ReferenceIndex {
    pub fn build(document: &ProjectDocument) -> Self {
        let mut index = Self::default();
        // Object definitions own every node inside their scope, so a
        // reference from an inner node names the definition's resource.
        for resource in document.resources.values() {
            let Some(scope) = resource
                .definition_scope
                .and_then(|id| document.definition_nodes.get(&id))
            else {
                continue;
            };
            let mut stack = vec![scope.id];
            while let Some(id) = stack.pop() {
                index.definition_owners.insert(id, resource.id);
                if let Some(node) = document.definition_nodes.get(&id)
                    && let NodeKind::Scope { children, .. } = &node.data
                {
                    stack.extend(children.iter().copied());
                }
            }
        }
        for (scene_id, scene) in &document.scenes {
            for node in scene.nodes.values() {
                index.add_node(*scene_id, node);
            }
        }
        for node in document.definition_nodes.values() {
            index.add_node_node(None, node);
        }
        for resource in document.resources.values() {
            for parameter in &resource.parameters {
                index.add_parameter(
                    &ReferenceLocation {
                        scene: None,
                        node: None,
                        resource: Some(resource.id),
                        via: ReferenceVia::Parameter(parameter.name.clone()),
                    },
                    parameter,
                );
            }
            if let Some(declared) = &resource.declared_in
                && let Some(graphics) = &declared.graphics
            {
                if let crate::MaterialBinding::Resource(id) = graphics.material {
                    index.add(
                        ItemId::Resource(id),
                        ReferenceLocation {
                            scene: None,
                            node: None,
                            resource: Some(resource.id),
                            via: ReferenceVia::Material,
                        },
                    );
                }
                for (id, via) in [
                    (graphics.inside_medium, ReferenceVia::MediumInside),
                    (graphics.outside_medium, ReferenceVia::MediumOutside),
                ] {
                    if let Some(id) = id {
                        index.add(
                            ItemId::Resource(id),
                            ReferenceLocation {
                                scene: None,
                                node: None,
                                resource: Some(resource.id),
                                via,
                            },
                        );
                    }
                }
            }
        }
        for refs in index.references.values_mut() {
            refs.sort_unstable();
            refs.dedup();
        }
        index
    }

    /// Referenced assets the document has no record of, each with the byte
    /// length a parameter array needs of it (0 when no array refers to it).
    pub(crate) fn unknown_assets(&self, document: &ProjectDocument) -> Vec<(crate::AssetId, u64)> {
        self.references
            .keys()
            .filter_map(|item| match item {
                ItemId::Asset(id) if !document.assets.contains_key(id) => {
                    Some((*id, self.array_bytes.get(id).copied().unwrap_or(0)))
                }
                _ => None,
            })
            .collect()
    }

    pub fn references(&self, item: ItemId) -> &[ReferenceLocation] {
        self.references.get(&item).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn count(&self, item: ItemId) -> usize {
        self.references(item).len()
    }

    /// The definition-owning resource of a node inside an
    /// ObjectDefinition scope (the "definition_node" reference's owner).
    pub fn definition_owner(&self, node: NodeId) -> Option<ResourceId> {
        self.definition_owners.get(&node).copied()
    }

    fn add_node(&mut self, scene: SceneId, node: &Node) {
        self.add_node_node(Some(scene), node);
    }

    fn add_node_node(&mut self, scene: Option<SceneId>, node: &Node) {
        if let NodeKind::Instance { definition } = &node.data {
            self.add(
                ItemId::Resource(*definition),
                ReferenceLocation {
                    scene,
                    node: Some(node.id),
                    resource: None,
                    via: ReferenceVia::Instance,
                },
            );
        }
        if let NodeKind::State { operation } = &node.data {
            match operation {
                StateOperation::Material(crate::MaterialBinding::Resource(id)) => {
                    let location = ReferenceLocation {
                        scene,
                        node: Some(node.id),
                        resource: None,
                        via: ReferenceVia::Material,
                    };
                    self.add(ItemId::Resource(*id), location);
                }
                StateOperation::Medium { inside, outside } => {
                    for (id, via) in [
                        (*inside, ReferenceVia::MediumInside),
                        (*outside, ReferenceVia::MediumOutside),
                    ] {
                        if let Some(id) = id {
                            self.add(
                                ItemId::Resource(id),
                                ReferenceLocation {
                                    scene,
                                    node: Some(node.id),
                                    resource: None,
                                    via,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        let params = match &node.data {
            NodeKind::Entity { parameters, .. } | NodeKind::Configuration { parameters, .. } => {
                parameters.as_slice()
            }
            _ => &[],
        };
        for parameter in params {
            self.add_parameter(
                &ReferenceLocation {
                    scene,
                    node: Some(node.id),
                    resource: None,
                    via: ReferenceVia::Parameter(parameter.name.clone()),
                },
                parameter,
            );
        }
    }

    fn add_parameter(&mut self, location: &ReferenceLocation, parameter: &Parameter) {
        match &parameter.values {
            ParameterValues::Assets(ids) => {
                for id in ids {
                    self.add(ItemId::Asset(*id), location.clone());
                }
            }
            ParameterValues::Array {
                asset,
                encoding,
                count,
            } => {
                let stride = match encoding {
                    crate::ArrayEncoding::F64Le | crate::ArrayEncoding::I64Le => 8,
                    crate::ArrayEncoding::BoolU8 => 1,
                };
                self.array_bytes
                    .insert(*asset, count.saturating_mul(stride));
                self.add(ItemId::Asset(*asset), location.clone());
            }
            ParameterValues::Resources { ids, .. } => {
                for id in ids {
                    self.add(ItemId::Resource(*id), location.clone());
                }
            }
            _ => {}
        }
    }

    fn add(&mut self, item: ItemId, location: ReferenceLocation) {
        self.references.entry(item).or_default().push(location);
    }
}
