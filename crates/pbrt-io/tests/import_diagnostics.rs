//! Import diagnostics carry explicit severities: every warning a
//! skipped or malformed piece produces says `Warning`, the importer's
//! automatic coordinate fix stays `Info`, budget errors name their limit
//! and dependency messages read as reasons.
use pbrt_io::{ImportOptions, import_project};
use project::*;
use std::fs;

fn import_text(text: &str) -> Vec<ImportDiagnostic> {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("scene.pbrt"), text).unwrap();
    let (_, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    session
        .document()
        .imports
        .values()
        .flat_map(|import| import.diagnostics.clone())
        .collect()
}

fn severities<'a>(diagnostics: &'a [ImportDiagnostic], code: &str) -> Vec<&'a ImportDiagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == code)
        .collect()
}

#[test]
fn skipped_directives_unresolved_includes_and_syntax_errors_are_warnings() {
    let diagnostics = import_text(
        "WorldBegin\n\
         Shape \"teapot\"\n\
         Include \"missing.pbrt\"\n\
         Shape 42 \"sphere\"\n\
         Shape \"sphere\"\n",
    );
    assert!(
        !diagnostics.is_empty(),
        "the scene must produce diagnostics"
    );
    for (code, count) in [("unsupported", 1), ("unresolved_include", 1), ("syntax", 1)] {
        let found = severities(&diagnostics, code);
        assert_eq!(found.len(), count, "code {code}: {diagnostics:#?}");
        for diagnostic in found {
            assert_eq!(
                diagnostic.severity,
                Some(ImportSeverity::Warning),
                "code {code}: {diagnostic:#?}"
            );
        }
    }
    // The dependency reason reads: what was rejected, and what it costs.
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Shape \"teapot\" is not supported by the importer (the shape is dropped)")),
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("missing.pbrt")),
        "{diagnostics:#?}"
    );
}

#[test]
fn coordinate_system_fixed_is_info() {
    let diagnostics = import_text(
        "WorldBegin\n\
         MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"]\n\
         AttributeBegin\n\
         NamedMaterial \"gold\"\n\
         Translate 1 2 3\n\
         CoordinateSystem \"matsys\"\n\
         AttributeEnd\n\
         CoordSysTransform \"matsys\"\n\
         Shape \"sphere\" \"float radius\" [0.5]\n",
    );
    let fixed = severities(&diagnostics, "coordinate_system_fixed");
    assert!(!fixed.is_empty(), "{diagnostics:#?}");
    for diagnostic in fixed {
        assert_eq!(
            diagnostic.severity,
            Some(ImportSeverity::Info),
            "{diagnostic:#?}"
        );
    }
}

#[test]
fn budget_errors_name_their_limit() {
    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join("scene.pbrt"),
        "WorldBegin\nShape \"sphere\"\nShape \"sphere\"\nShape \"sphere\"\n",
    )
    .unwrap();
    let error = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &ImportOptions {
            max_nodes: 2,
            ..ImportOptions::default()
        },
        || false,
    )
    .err()
    .expect("the directive budget must fail this import");
    assert!(
        error
            .to_string()
            .contains("more than 2 directives (import limit)"),
        "unexpected error: {error}"
    );
}

/// The per-directive token budget is an import limit like the others
/// and names itself; a bulk density directive (smoke-plume's runs to
/// 9.4 million values) must fit under the default. At the limit the
/// directive imports; one token under it the budget refuses.
#[test]
fn the_directive_token_budget_is_a_named_import_limit() {
    // `Shape "sphere" "float radius" [ 1 1 1 1 1 1 1 1 1 ]` is 14 tokens;
    // WorldBegin is 1 and `Shape "disk"` is 2, both far under any limit.
    let values = "1 1 1 1 1 1 1 1 1";
    let text =
        format!("WorldBegin\nShape \"sphere\" \"float radius\" [{values}]\nShape \"disk\"\n");
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("scene.pbrt"), &text).unwrap();
    for (limit, allowed) in [(13, false), (14, true)] {
        let result = import_project(
            &home.path().join("scene.pbrt"),
            &home.path().join(format!("project-{limit}")),
            &ImportOptions {
                max_directive_tokens: limit,
                ..ImportOptions::default()
            },
            || false,
        );
        match (allowed, result) {
            (true, Ok(_)) => {}
            (true, Err(error)) => panic!("a 14-token directive must import at limit 14: {error}"),
            (false, Err(error)) => assert!(
                error
                    .to_string()
                    .contains("more than 13 tokens in one directive (import limit)"),
                "unexpected error: {error}"
            ),
            (false, Ok(_)) => panic!("a 14-token directive must refuse at limit 13"),
        }
    }
    // smoke-plume's density directive (9,437,184 values plus the grid
    // extents and the parent-continued spectra) fits the default budget.
    assert_eq!(ImportOptions::default().max_directive_tokens, 16_000_000);
}
