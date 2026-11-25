//! Golden hashes of the text export. The direct-preview pipeline (see
//! routes through the shared emitter; these constants pin the canonicalized
//! scene.pbrt so any formatting drift fails before it can ship.
//!
//! Canonicalization: imports assign fresh UUIDs, so `resource_{uuid}` and
//! `assets/{uuid}-name` tokens are replaced with first-appearance ordinals
//! (`<u0>`, `<u1>`, ...) before hashing. Traversal order is deterministic,
//! so the same structure always canonicalizes to the same text.
use pbrt_io::{ImportOptions, export_project, import_project};
use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}
/// Replace the `resource_{uuid}` / `assets/{uuid}` UUIDs with first-appearance
/// ordinals so fresh imports of the same scene hash identically. Traversal
/// order is deterministic, so the numbering (and the hash) is stable.
fn canonicalize(bytes: &[u8]) -> String {
    let text = String::from_utf8(bytes.to_vec()).expect("scene.pbrt must be UTF-8");
    let chars: Vec<char> = text.chars().collect();
    let mut ordinals = std::collections::BTreeMap::<String, usize>::new();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if is_uuid_at(&chars, index) {
            let uuid: String = chars[index..index + 36].iter().collect();
            let next = ordinals.len();
            let slot = *ordinals.entry(uuid).or_insert(next);
            out.push_str(&format!("<u{slot}>"));
            index += 36;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

/// Canonical 8-4-4-4-12 lowercase hex spelling at `index`.
fn is_uuid_at(chars: &[char], index: usize) -> bool {
    if index + 36 > chars.len() {
        return false;
    }
    for (offset, c) in chars[index..index + 36].iter().enumerate() {
        match offset {
            8 | 13 | 18 | 23 => {
                if *c != '-' {
                    return false;
                }
            }
            _ => {
                if !is_hex(*c) {
                    return false;
                }
            }
        }
    }
    true
}

fn golden(source: &str, expected: &str) {
    let temp = tempfile::tempdir().unwrap();
    let (store, session) = import_project(
        &fixture(source),
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let bytes = fs::read(&exported).unwrap();
    let actual = format!("{:x}", Sha256::digest(canonicalize(&bytes).as_bytes()));
    println!("NEWHASH {source} {actual}");
    if std::env::var("PBRT_UI_GOLDEN_IGNORE").is_ok() {
        return;
    }
    assert_eq!(
        actual, expected,
        "{source}: scene.pbrt drifted from the golden hash (P5 violation); \
         investigate before touching any emitter logic"
    );
}

// 2026-11-07: hashes repinned for the deliberate 2026-11-05 "persist
// explicit scene render camera selection and settings" emission format
// (the selected camera's start/end Transform pair plus its own Camera
// directive at the end of Options) and for no longer double-emitting that
// Camera node in the generic options pass (export/import idempotency).
// For matrix-preview this also moves the first Transform in the text to
// the camera block; the fixture-matrix pins live in
// direct_preview_parity::matrix_and_animation_directives_keep_their_lexical_shape.
// 2026-11-09: repinned for transform-parent resolution (identity/coordinate
// resets resolved at import and pose-based camera emission: a
// posed static camera now writes a single inverse-pose `Transform` instead of
// the start/end ActiveTransform pair.
#[test]
fn export_text_is_pinned_by_golden_hashes() {
    // Hashes are content pins, not secrets: any intentional change to the
    // emission format must update them in the same commit with a reason.
    golden(
        "minimal.pbrt",
        "c0fb52b9abd62ab715dc6158f15c2db8e20aa0b88f08a3d41bd75a09a8521382",
    );
    golden(
        "editor-preview.pbrt",
        "e6ab849212892d7c93c3df2182d8d09449dabf4bae4d4cbae84dbfc109905f56",
    );
    golden(
        "instances-preview.pbrt",
        "3861dab0faf017391b085cb811c160751a450fb6cae23008b6c76b6cfcd7c4ca",
    );
    golden(
        "render-preview.pbrt",
        "eb044919912d3fa999235a045930bbf0efcde92af24cd2405f12ec1e8e3a402c",
    );
    golden(
        "matrix-preview.pbrt",
        "3b2e23e7f8a39b405bf7e665be5bc3f8504f882831e31198d88b673c17ed56e7",
    );
}
