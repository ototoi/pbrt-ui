//! Shared placement rules for `Assets/` items: label
//! sanitizing, the `_N` numbering and the per-kind default folders. Import
//! and resource creation share these; the desktop additionally has its
//! ` (N)` display form, which is a screen-only rule handled by the UI.
use project::layout::{LocationNamer, fallback_stem, kind_extension, kind_folder, sanitize};

#[test]
fn sanitize_replaces_path_characters_and_keeps_the_rest() {
    assert_eq!(sanitize("Copper bowl", "material"), "Copper bowl");
    assert_eq!(
        sanitize("a/b\\c:d*e?f\"g<h>i|j", "material"),
        "a_b_c_d_e_f_g_h_i_j"
    );
    // Control characters fold to underscores as well.
    assert_eq!(sanitize("bad\u{7}name", "material"), "bad_name");
    // Trailing dots cannot end a name; surrounding space folds away.
    assert_eq!(sanitize("  spaced.  ", "material"), "spaced");
    // Empty, lone-dot and dotdot names fall back.
    assert_eq!(sanitize("", "material"), "material");
    assert_eq!(sanitize(".", "material"), "material");
    assert_eq!(sanitize("..", "material"), "material");
}

#[test]
fn fallback_stems_keep_imports_readable() {
    assert_eq!(fallback_stem("material"), "material");
    assert_eq!(fallback_stem("texture"), "texture");
    assert_eq!(fallback_stem("medium"), "medium");
    assert_eq!(fallback_stem("object"), "object");
    assert_eq!(fallback_stem("scene"), "scene");
    assert_eq!(fallback_stem("array"), "array");
    assert_eq!(fallback_stem("unknown"), "file");
}

#[test]
fn locations_fold_case_and_number_with_underscores() {
    let mut namer = LocationNamer::new();
    assert_eq!(
        namer.take("Materials", " Copper ", "material"),
        "Materials/Copper.material"
    );
    // Same label again: append `_N` (the display-side ` (N)` is the
    // desktop's own rule and deliberately different).
    assert_eq!(
        namer.take("Materials", "Copper", "material"),
        "Materials/Copper_1.material"
    );
    // Case-insensitive collisions count.
    assert_eq!(
        namer.take("Materials", "copper", "material"),
        "Materials/copper_2.material"
    );
    // Distinct labels stay clean.
    assert_eq!(
        namer.take("Materials", "Diffuse", "material"),
        "Materials/Diffuse.material"
    );
    // Folders and other extensions keep working.
    assert_eq!(namer.take("Scenes", "Night", "scene"), "Scenes/Night.scene");
    // A label that folds to a single forbidden character keeps it (imports
    // behaved the same); one that sanitizes away uses the fallback stem.
    assert_eq!(namer.take("Media", "?", "medium"), "Media/_.medium");
    assert_eq!(namer.take("Media", " ", "medium"), "Media/medium.medium");
}

#[test]
fn kinds_own_their_default_folders_and_extensions() {
    for (kind, folder, extension) in [
        (project::ResourceKind::Material, "Materials", "material"),
        (project::ResourceKind::Texture, "Textures", "texture"),
        (project::ResourceKind::Medium, "Media", "medium"),
        (project::ResourceKind::ObjectDefinition, "Objects", "object"),
    ] {
        assert_eq!(kind_folder(kind), folder);
        assert_eq!(kind_extension(kind), extension);
    }
}
