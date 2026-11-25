//! PBRT input generation for an explicit render camera.
use super::*;
pub(super) fn write_view_scene(
    root: &str,
    view: &protocol::view::ViewState,
    directory: &Path,
) -> Result<PathBuf, ErrorPayload> {
    use scene_parse::{Value as ParsedValue, document::read_document};
    let document = read_document(Path::new(root)).map_err(fail)?;
    let camera = &view.camera;
    let aspect = view.size[0] as f64 / view.size[1] as f64;
    let camera_directive = format!(
        "ActiveTransform All\nIdentity\n{}LookAt {} {} {} {} {} {} {} {} {}\nMediumInterface \"\" \"\"\nCamera \"perspective\" \"float fov\" [{}] \"float screenwindow\" [{} {} -1 1] \"float lensradius\" [0] \"float shutteropen\" [{}] \"float shutterclose\" [{}]\n",
        // A right-handed scene's mirror, as its own cameras carry it.
        if camera.mirror { "Scale -1 1 1\n" } else { "" },
        camera.eye[0],
        camera.eye[1],
        camera.eye[2],
        camera.target[0],
        camera.target[1],
        camera.target[2],
        camera.up[0],
        camera.up[1],
        camera.up[2],
        camera.fov_y_degrees,
        -aspect,
        aspect,
        view.time,
        view.time,
    );
    let mut output = String::new();
    if !document
        .directives
        .iter()
        .any(|directive| directive.name == "Camera")
    {
        output.push_str(&camera_directive);
    }
    if !document
        .directives
        .iter()
        .any(|directive| directive.name == "Film")
    {
        output.push_str(&format!(
            "Film \"rgb\" \"integer xresolution\" [{}] \"integer yresolution\" [{}]\n",
            view.size[0], view.size[1],
        ));
    }
    let mut world = false;
    let original_parent = Path::new(root).parent().unwrap();
    for d in document.directives {
        if d.name == "WorldBegin" {
            world = true;
        }
        if !world
            && matches!(
                d.name.as_str(),
                "LookAt"
                    | "Translate"
                    | "Scale"
                    | "Rotate"
                    | "Transform"
                    | "ConcatTransform"
                    | "Identity"
                    | "ActiveTransform"
                    | "CoordSysTransform"
            )
        {
            continue;
        }
        if d.name == "Camera" {
            output.push_str(&camera_directive);
            continue;
        }
        output.push_str(&d.name);
        let bracket = matches!(d.name.as_str(), "Transform" | "ConcatTransform");
        if bracket {
            output.push_str(" [");
        }
        for token in &d.positional_tokens {
            output.push(' ');
            output.push_str(token);
        }
        if bracket {
            output.push_str(" ]");
        }
        for parameter in d.parameters {
            if d.name == "Film"
                && matches!(
                    parameter.name.as_str(),
                    "xresolution" | "yresolution" | "cropwindow" | "pixelbounds"
                )
            {
                continue;
            }
            output.push(' ');
            output.push_str(
                &serde_json::to_string(&format!("{} {}", parameter.value_type, parameter.name))
                    .unwrap(),
            );
            output.push_str(" [");
            for (v, token) in parameter.values.into_iter().zip(parameter.value_tokens) {
                let token = if let ParsedValue::Text(text) = v {
                    let is_path = parameter.name == "filename"
                        || parameter.name == "normalmap"
                        || parameter.name == "lensfile"
                        || (parameter.value_type == "spectrum"
                            && (text.contains('/') || Path::new(&text).extension().is_some()));
                    if is_path && !Path::new(&text).is_absolute() {
                        let local = Path::new(&d.path).parent().unwrap().join(&text);
                        let fallback = original_parent.join(&text);
                        serde_json::to_string(
                            if local.exists() { local } else { fallback }
                                .to_str()
                                .ok_or_else(|| fail("non UTF-8 asset path"))?,
                        )
                        .map_err(fail)?
                    } else {
                        token
                    }
                } else {
                    token
                };
                output.push(' ');
                output.push_str(&token);
            }
            output.push_str(" ]");
        }
        if d.name == "Film" {
            output.push_str(&format!(
                " \"integer xresolution\" [{}] \"integer yresolution\" [{}] \"float cropwindow\" [0 1 0 1]",
                view.size[0], view.size[1],
            ));
        }
        output.push('\n');
    }
    let path = directory.join("view-scene.pbrt");
    fs::write(&path, output).map_err(fail)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render View of a mirrored (right-handed) view puts the same
    /// `Scale -1 1 1` before its LookAt that the scene's cameras carry, so
    /// the render matches the viewport.
    #[test]
    fn mirrored_views_write_the_camera_mirror() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("scene.pbrt");
        fs::write(&root, "Camera \"perspective\"\nWorldBegin\n").unwrap();
        let mut view = protocol::view::initial_view(None);
        let written = |view: &protocol::view::ViewState| {
            let path = write_view_scene(root.to_str().unwrap(), view, temp.path()).unwrap();
            fs::read_to_string(path).unwrap()
        };
        assert!(!written(&view).contains("Scale -1 1 1"));
        view.camera.mirror = true;
        assert!(written(&view).contains("Identity\nScale -1 1 1\nLookAt "));
    }
}
