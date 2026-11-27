//! Dock layout version 4: the default arrangement and where a closed panel
//! comes back. Pure so the rules can be tested without a window.
use super::PanelKind;
use serde_json::{Value, json};
use std::path::Path;

/// Saved layouts from version 3 or earlier reset to the default arrangement.
pub const LAYOUT_VERSION: usize = 4;

pub fn accepts_saved_layout(version: Option<usize>) -> bool {
    version == Some(LAYOUT_VERSION)
}

/// The Scene panel is the only native viewport today. Keep its preference in
/// a sibling file to `layout.json`, keyed by viewport name so additional
/// viewport panels can retain independent state later.
pub fn overlays_visible(bytes: &[u8], viewport: &str) -> Option<bool> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    (value["version"] == 1)
        .then(|| value["viewports"][viewport]["overlays"].as_bool())
        .flatten()
}

pub fn save_overlays(path: &Path, viewport: &str, visible: bool) -> std::io::Result<()> {
    let mut value = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(|value| value["version"] == 1)
        .unwrap_or_else(|| json!({"version": 1, "viewports": {}}));
    value["viewports"][viewport]["overlays"] = json!(visible);
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)
}

/// The tab groups of the default arrangement:
///
/// ```text
/// h_split
/// ├─ tabs[Hierarchy] 235px
/// ├─ v_split flex
/// │ ├─ h_split
/// │ │ ├─ tabs[Scene] flex
/// │ │ └─ tabs[Render] flex
/// │ └─ tabs[Contents, Console, Jobs] 180px
/// └─ tabs[Inspector] 330px
/// ```
pub const DEFAULT_GROUPS: [&[PanelKind]; 5] = [
    &[PanelKind::Hierarchy],
    &[PanelKind::Scene],
    &[PanelKind::Render],
    &[PanelKind::Contents, PanelKind::Console, PanelKind::Jobs],
    &[PanelKind::Inspector],
];

/// Panels sharing `kind`'s default tab group, in group order. A reopened panel
/// joins the group of the first of these still in the dock; with none left it
/// becomes a new tab in the center.
pub fn default_siblings(kind: PanelKind) -> Vec<PanelKind> {
    DEFAULT_GROUPS
        .iter()
        .find(|group| group.contains(&kind))
        .map(|group| {
            group
                .iter()
                .copied()
                .filter(|other| *other != kind)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_v3_resets_to_the_default_arrangement() {
        assert!(!accepts_saved_layout(Some(3)));
        assert!(!accepts_saved_layout(None));
        assert!(accepts_saved_layout(Some(4)));
    }

    #[test]
    fn viewport_overlay_preferences_are_independent_and_versioned() {
        let bytes =
            br#"{"version":1,"viewports":{"Scene":{"overlays":false},"Other":{"overlays":true}}}"#;
        assert_eq!(overlays_visible(bytes, "Scene"), Some(false));
        assert_eq!(overlays_visible(bytes, "Other"), Some(true));
        assert_eq!(overlays_visible(bytes, "Missing"), None);
        assert_eq!(
            overlays_visible(
                br#"{"version":2,"viewports":{"Scene":{"overlays":false}}}"#,
                "Scene"
            ),
            None
        );
    }

    #[test]
    fn saving_one_viewport_preserves_the_others() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("viewport-overlays.json");
        save_overlays(&path, "Scene", false).unwrap();
        save_overlays(&path, "Other", true).unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(overlays_visible(&bytes, "Scene"), Some(false));
        assert_eq!(overlays_visible(&bytes, "Other"), Some(true));
    }

    #[test]
    fn every_panel_has_one_default_group() {
        for kind in PanelKind::ALL {
            let groups = DEFAULT_GROUPS
                .iter()
                .filter(|group| group.contains(&kind))
                .count();
            assert_eq!(groups, 1, "{}", kind.name());
        }
    }

    #[test]
    fn a_reopened_panel_rejoins_its_default_group() {
        assert_eq!(
            default_siblings(PanelKind::Jobs),
            [PanelKind::Contents, PanelKind::Console]
        );
        assert!(default_siblings(PanelKind::Inspector).is_empty());
    }
}
