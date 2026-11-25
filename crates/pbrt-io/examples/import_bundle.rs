use std::{error::Error, path::PathBuf};
fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: import_bundle SOURCE.pbrt NEW_PROJECT_DIR NEW_BUNDLE_DIR".into());
    }
    let (store, session) = pbrt_io::import_project(
        &args[0],
        &args[1],
        &pbrt_io::ImportOptions::default(),
        || false,
    )?;
    let scene = pbrt_io::export_project(&session.snapshot(), &store, &args[2], || false)?;
    println!(
        "project: {}\nscene: {}\nnodes: {}\nassets: {}",
        store.directory().display(),
        scene.display(),
        session.document().default_scene_view()?.scene.nodes.len(),
        session.document().assets.len()
    );
    Ok(())
}
