use scene_parse::{ParseOptions, parse_scene};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("pbrt-ui-preview: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let scene = args
        .next()
        .ok_or("usage: pbrt-ui-preview SCENE.pbrt [OUTPUT_DIR]")?;
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("pbrt-ui-preview"));
    if args.next().is_some() {
        return Err("usage: pbrt-ui-preview SCENE.pbrt [OUTPUT_DIR]".into());
    }
    let preview = parse_scene(&scene, ParseOptions::default())?;
    preview.write_to(&output)?;
    let mut diagnostic_counts = BTreeMap::new();
    for diagnostic in &preview.diagnostics {
        *diagnostic_counts
            .entry(diagnostic.code.clone())
            .or_insert(0usize) += 1;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "scene": protocol::path::canonicalize(scene)?.display().to_string(),
            "output": protocol::path::canonicalize(output)?.display().to_string(),
            "mesh_count": preview.manifest.meshes.len(),
            "draw_count": preview.manifest.draws.len(),
        "unsupported_count": preview.diagnostics.iter().filter(|d| d.code.contains("unsupported")).count(),
        "diagnostic_counts": diagnostic_counts,
        "diagnostics_sample": preview.diagnostics.iter().take(20).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}
