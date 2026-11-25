use project::*;

#[test]
fn detached_scene_imports_have_fresh_ids_and_need_no_input_for_redo() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("scene.pbrt");
    std::fs::write(&source, "WorldBegin\nMakeNamedMaterial \"Mat\" \"string type\" \"diffuse\"\nNamedMaterial \"Mat\"\nShape \"sphere\"\nWorldEnd\n").unwrap();
    let first = pbrt_io::prepare_scene(&source, &Default::default(), || false)
        .unwrap()
        .0;
    let second = pbrt_io::prepare_scene(&source, &Default::default(), || false)
        .unwrap()
        .0;
    assert_ne!(
        first.document().scenes.keys().next(),
        second.document().scenes.keys().next()
    );
    assert_ne!(
        first.document().resources.keys().next(),
        second.document().resources.keys().next()
    );
    assert_ne!(
        first.document().imports.keys().next(),
        second.document().imports.keys().next()
    );
    let mut target = ProjectSession::new(ProjectDocument::new_empty("Target")).unwrap();
    let first = first.placed_for(target.document(), None).unwrap();
    target
        .apply(&target.revision(), &[Edit::ImportBatch { batch: first }])
        .unwrap();
    let second = second.placed_for(target.document(), None).unwrap();
    assert_eq!(
        second.document().scenes.values().next().unwrap().location,
        "Scenes/scene_1.scene"
    );
    assert_eq!(
        second
            .document()
            .resources
            .values()
            .next()
            .unwrap()
            .location,
        "Materials/Mat_1.material"
    );
    target
        .apply(&target.revision(), &[Edit::ImportBatch { batch: second }])
        .unwrap();
    assert!(target.active_scene().is_none());
    assert!(target.document().default_scene.is_none());
    std::fs::remove_file(&source).unwrap();
    target.undo(&target.revision()).unwrap();
    assert_eq!(target.document().scenes.len(), 1);
    target.redo(&target.revision()).unwrap();
    assert_eq!(target.document().scenes.len(), 2);
}

#[test]
fn scene_addition_requires_a_world_and_honors_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bad.pbrt");
    std::fs::write(&path, "Film \"rgb\"\n").unwrap();
    assert!(pbrt_io::prepare_scene(&path, &Default::default(), || false).is_err());
    std::fs::write(&path, "WorldBegin\nWorldEnd\n").unwrap();
    assert!(pbrt_io::prepare_scene(&path, &Default::default(), || true).is_err());
    assert!(pbrt_io::prepare_scene(&path, &Default::default(), || false).is_ok());
}

#[test]
fn imagemaps_sharing_an_image_reference_one_asset_placed_in_images() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("wood.exr"), b"image bytes").unwrap();
    let source = directory.path().join("scene.pbrt");
    std::fs::write(
        &source,
        "WorldBegin\n\
         Texture \"wood\" \"spectrum\" \"imagemap\" \"string filename\" [\"wood.exr\"]\n\
         Texture \"wood_rough\" \"float\" \"imagemap\" \"string filename\" [\"wood.exr\"] \"float scale\" [0.5]\n\
         Shape \"sphere\"\nWorldEnd\n",
    )
    .unwrap();
    let prepared = pbrt_io::prepare_scene(&source, &Default::default(), || false)
        .unwrap()
        .0;
    let target = ProjectDocument::new_empty("Target");
    let placed = prepared.placed_for(&target, None).unwrap();
    let document = placed.document();
    let [image] = document.assets.values().collect::<Vec<_>>()[..] else {
        panic!("one image asset expected: {:?}", document.assets);
    };
    assert_eq!(image.location, "Images/wood.exr");
    let mut locations = document
        .resources
        .values()
        .map(|resource| {
            assert_eq!(
                resource
                    .parameters
                    .iter()
                    .find(|p| p.name == "filename")
                    .unwrap()
                    .values,
                ParameterValues::Assets(vec![image.id])
            );
            resource.location.as_str()
        })
        .collect::<Vec<_>>();
    locations.sort();
    assert_eq!(
        locations,
        ["Textures/wood.texture", "Textures/wood_rough.texture"]
    );
}
