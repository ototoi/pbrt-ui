//! Dock layout version 4: the default arrangement and where a closed panel
//! comes back. Pure so the rules can be tested without a window.
use super::PanelKind;

/// Saved layouts from version 3 or earlier reset to the default arrangement.
pub const LAYOUT_VERSION: usize = 4;

pub fn accepts_saved_layout(version: Option<usize>) -> bool {
    version == Some(LAYOUT_VERSION)
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
