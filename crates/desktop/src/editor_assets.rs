//! Embed editor icons alongside the component library's icon catalog.
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    EditorIcons,
    [
        Box,
        ScanSearch,
        Triangle,
        Scan,
        Focus,
        PanelsTopLeft,
        Camera,
        Download,
        Image,
        Droplets,
        Palette,
        Boxes,
        FileBox,
        Eye,
        EyeOff
    ]
);

pub struct EditorAssets;

impl AssetSource for EditorAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match EditorIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            // `Assets` only embeds the component library's default subset, so
            // icons such as Save or FolderMinus would render blank. Fall back
            // to the complete catalog instead.
            None => gpui_kit::assets::AllAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::AllAssets.list(path)?;
        paths.extend(EditorIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::assets::IconName;

    #[test]
    fn editor_and_dock_icons_are_available_in_registered_assets() {
        for icon in [
            IconName::Box,
            IconName::ScanSearch,
            IconName::Triangle,
            IconName::Scan,
            IconName::Focus,
            IconName::PanelsTopLeft,
            IconName::FolderOpen,
            IconName::RefreshCw,
            IconName::ChevronDown,
            IconName::Camera,
            IconName::Download,
            IconName::Image,
            IconName::Droplets,
            IconName::Palette,
            IconName::Boxes,
            IconName::FileBox,
            IconName::Eye,
            IconName::EyeOff,
            // Toolbar icons that are not in the component default subset; they
            // must resolve through the complete catalog.
            IconName::Save,
            IconName::FolderMinus,
            IconName::Undo2,
            IconName::Redo2,
            // Jobs panel state icons and the Spinner's loader.
            IconName::Check,
            IconName::X,
            IconName::CircleSlash,
            IconName::Loader,
        ] {
            let path = icon.path();
            let bytes = EditorAssets.load(&path).unwrap().unwrap();
            assert!(
                std::str::from_utf8(&bytes).unwrap().contains("<svg"),
                "{path}"
            );
        }
    }
}
