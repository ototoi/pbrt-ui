//! The project-wide history contains one entry per
//! committed operation — its label for describing the undo, the touched
//! records, and the payload leases an undo of it needs. The undo list
//! keeps its 128-entry budget; evicted entries release their leases.
use crate::session::Delta;
use crate::{Edit, PayloadLease};
use std::collections::VecDeque;
use std::sync::Arc;

/// How the entries describe their operation: the edit kinds of the
/// transaction that produced them, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationLabel(Vec<String>);

impl OperationLabel {
    /// Label a committed edit transaction. An empty transaction is not
    /// recordable, but the shape stays for imports that name
    /// themselves differently.
    pub(crate) fn of_edits(edits: &[Edit]) -> Self {
        Self(edits.iter().map(Edit::kind).map(str::to_owned).collect())
    }

    /// The edit kinds, in transaction order — for describing the undo
    /// ("Undo rename node") and later wire/GUI labels.
    pub fn kinds(&self) -> &[String] {
        &self.0
    }
}

/// One committed operation in the project's shared history.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    /// What produced this delta, for undo/redo labels.
    label: OperationLabel,
    /// The touched records, restored by their recorded owner.
    pub(crate) delta: Delta,
    /// Staged asset bytes an undo of this entry re-materializes; dropped
    /// with the entry, releasing the last lease when no other owner holds it.
    pub(crate) payloads: Vec<Arc<PayloadLease>>,
}

impl HistoryEntry {
    pub(crate) fn new(
        label: OperationLabel,
        delta: Delta,
        payloads: Vec<Arc<PayloadLease>>,
    ) -> Self {
        Self {
            label,
            delta,
            payloads,
        }
    }

    /// The entry's operation label.
    pub fn label(&self) -> &OperationLabel {
        &self.label
    }
}

/// The project's single undo/redo stack for edits, imports, moves, renames,
/// and deletions.
#[derive(Default)]
pub struct History {
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

impl History {
    /// Record a committed operation: a new success drops the redo stack,
    /// and the undo list keeps at most `UNDO_LIMIT` entries.
    pub(crate) fn record(&mut self, entry: HistoryEntry) {
        self.redo.clear();
        self.undo.push_back(entry);
        while self.undo.len() > UNDO_LIMIT {
            self.undo.pop_front();
        }
    }

    /// The entry an undo would rewind, if any.
    pub(crate) fn undo_peek(&self) -> Option<&HistoryEntry> {
        self.undo.back()
    }

    /// Move the undone entry onto the redo stack (after a successful
    /// restore), returning it for the change report.
    pub(crate) fn shift_to_redo(&mut self) -> HistoryEntry {
        let entry = self.undo.pop_back().expect("undo stack was not empty");
        self.redo.push(entry.clone());
        entry
    }

    /// The entry a redo would reapply, if any.
    pub(crate) fn redo_peek(&self) -> Option<&HistoryEntry> {
        self.redo.last()
    }

    /// Move the redone entry back onto the undo stack (after a successful
    /// restore), returning it for the change report.
    pub(crate) fn shift_to_undo(&mut self) -> HistoryEntry {
        let entry = self.redo.pop().expect("redo stack was not empty");
        self.undo.push_back(entry.clone());
        entry
    }

    /// Every entry, oldest undo first then redo — for lease lookups.
    pub(crate) fn entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.undo.iter().chain(self.redo.iter())
    }

    /// Every entry, for re-homing their leases (Save As).
    pub(crate) fn entries_mut(&mut self) -> impl Iterator<Item = &mut HistoryEntry> {
        self.undo.iter_mut().chain(self.redo.iter_mut())
    }

    /// Undoable entries, oldest first — for tests and later cheap counts.
    #[cfg(test)]
    fn undo_len(&self) -> usize {
        self.undo.len()
    }

    #[cfg(test)]
    fn redo_len(&self) -> usize {
        self.redo.len()
    }

    #[cfg(test)]
    fn redo_is_empty(&self) -> bool {
        self.redo.is_empty()
    }
}

const UNDO_LIMIT: usize = 128;

impl Edit {
    /// The wire word for one edit kind, for operation labels.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::CreateCamera { .. } => "create_camera",
            Self::CreateConfiguration { .. } => "create_configuration",
            Self::SetRenderCamera { .. } => "set_render_camera",
            Self::SetSceneCoordinateSystem { .. } => "set_scene_coordinate_system",
            Self::ImportBatch { .. } => "import",
            Self::CreateScene { .. } => "create_scene",
            Self::DuplicateScene { .. } => "duplicate_scene",
            Self::DeleteScene { .. } => "delete_scene",
            Self::MoveItem { .. } => "move_item",
            Self::CreateFolder { .. } => "create_folder",
            Self::MoveFolder { .. } => "move_folder",
            Self::DeleteFolder { .. } => "delete_folder",
            Self::DeleteItem { .. } => "delete_item",
            Self::SetAssetReference { .. } => "set_asset_reference",
            Self::CreatePreset { .. } => "create_preset",
            Self::CreateResource { .. } => "create_resource",
            Self::MakeObjectDefinition { .. } => "make_object_definition",
            Self::UnpackObjectInstance { .. } => "unpack_object_instance",
            Self::SetAreaLight { .. } => "set_area_light",
            Self::ClearLocalBinding { .. } => "clear_local_binding",
            Self::SetReverseOrientation { .. } => "set_reverse_orientation",
            Self::SetLocalTransform { .. } => "set_local_transform",
            Self::SetLocalBinding { .. } => "set_local_binding",
            Self::RenameNode { .. } => "rename_node",
            Self::SetNodeActive { .. } => "set_node_active",
            Self::RenameResource { .. } => "rename_resource",
            Self::SetTextureValueType { .. } => "set_texture_value_type",
            Self::SetTransform { .. } => "set_transform",
            Self::SetMaterialBinding { .. } => "set_material_binding",
            Self::SetInstanceDefinition { .. } => "set_instance_definition",
            Self::DeleteNode { .. } => "delete_node",
            Self::DuplicateNode { .. } => "duplicate_node",
            Self::MoveNode { .. } => "move_node",
            Self::SetParameter { .. } => "set_parameter",
            Self::SetParameterValues { .. } => "set_parameter_values",
            Self::SetImplementation { .. } => "set_implementation",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeId;

    #[test]
    fn history_records_labels_and_keeps_its_budget() {
        let mut history = History::default();
        for index in 0..130 {
            history.record(HistoryEntry::new(
                OperationLabel::of_edits(&[Edit::RenameNode {
                    node: NodeId::new(),
                    label: format!("Edit {index}"),
                }]),
                Delta::default(),
                Vec::new(),
            ));
        }
        assert_eq!(
            history.undo_len(),
            128,
            "the oldest entries fall off the back"
        );
        // Recording a new edit after undo discards the redo stack.
        let undone = history.shift_to_redo();
        assert_eq!(history.undo_len(), 127);
        assert_eq!(history.redo_len(), 1);
        assert_eq!(undone.label().kinds(), ["rename_node".to_owned()]);
        history.record(HistoryEntry::new(
            OperationLabel::of_edits(&[]),
            Delta::default(),
            Vec::new(),
        ));
        assert!(history.redo_is_empty());
    }

    #[test]
    fn labels_name_the_edit_kinds_in_order() {
        let label = OperationLabel::of_edits(&[
            Edit::CreateScene {
                location: "Scenes/extra.scene".into(),
            },
            Edit::RenameNode {
                node: NodeId::new(),
                label: "Renamed".into(),
            },
        ]);
        assert_eq!(
            label.kinds(),
            ["create_scene".to_owned(), "rename_node".to_owned()]
        );
    }
}
