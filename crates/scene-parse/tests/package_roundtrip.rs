use scene_parse::{
    GeometryCache, ParseOptions, PreviewProgram, document::read_document, preview_from_program,
    read_preview_blobs, read_preview_diagnostics, read_preview_manifest, read_preview_package,
    write_preview, write_preview_with_store,
};
use std::{collections::BTreeMap, fs};

/// The direct-preview writer and the capture/pick reader share one package
/// format; a round trip must return exactly the manifest, blobs and
/// diagnostics that were written.
#[test]
fn direct_preview_package_round_trips_through_the_shared_reader() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\" \"float radius\" [1]\nShape \"disk\"\n",
    )
    .unwrap();
    let document = read_document(&source).unwrap();
    let program = PreviewProgram {
        directives: document.directives,
        origins: BTreeMap::new(),
        sources: BTreeMap::new(),
    };
    let preview = preview_from_program(
        program,
        None,
        dir.path(),
        ParseOptions::default(),
        &mut GeometryCache::default(),
    )
    .unwrap();
    assert_eq!(preview.manifest.draws.len(), 2);

    let package = dir.path().join("package");
    write_preview(&preview, &package, true).unwrap();
    let read = read_preview_package(&package).unwrap();
    assert_eq!(read.manifest, preview.manifest);
    assert_eq!(read.blobs, preview.blobs);
    assert_eq!(read.diagnostics, preview.diagnostics);
}

/// The desktop reads only the blobs it needs. The split
/// readers must return the manifest alone, exactly the requested blobs,
/// and the diagnostics log — and a blob the requester did not name is
/// never opened (deleting the rest breaks the whole-package reader but
/// not the partial read of the surviving names).
#[test]
fn partial_package_reads_only_the_named_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\" \"float radius\" [1]\nShape \"disk\"\n",
    )
    .unwrap();
    let document = read_document(&source).unwrap();
    let program = PreviewProgram {
        directives: document.directives,
        origins: BTreeMap::new(),
        sources: BTreeMap::new(),
    };
    let preview = preview_from_program(
        program,
        None,
        dir.path(),
        ParseOptions::default(),
        &mut GeometryCache::default(),
    )
    .unwrap();
    let package = dir.path().join("package");
    write_preview(&preview, &package, true).unwrap();

    // Names must come from the freshly read manifest, not the writer's.
    let manifest = read_preview_manifest(&package).unwrap();
    assert_eq!(manifest, preview.manifest);
    let all: std::collections::BTreeSet<String> = preview.blobs.keys().cloned().collect();
    let wanted: std::collections::BTreeSet<String> = all.iter().take(1).cloned().collect();
    for name in all.iter().skip(1) {
        fs::remove_file(package.join(name)).unwrap();
    }
    let blobs = read_preview_blobs(&package, &manifest, &wanted).unwrap();
    let expected: BTreeMap<String, Vec<u8>> = preview
        .blobs
        .iter()
        .filter(|(name, _)| wanted.contains(*name))
        .map(|(name, bytes)| (name.clone(), bytes.clone()))
        .collect();
    assert_eq!(blobs, expected);
    // The diagnostics split with the manifest: capture/pick read it via the
    // whole-package reader, the desktop install via this one.
    assert_eq!(
        read_preview_diagnostics(&package).unwrap(),
        preview.diagnostics
    );
    // The whole-package reader still touches every blob, so the gap the
    // partial read ignored is a real gap on its side.
    assert!(read_preview_package(&package).is_err());
}

#[test]
fn content_store_reuses_mesh_blob_without_copying_cached_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\" \"float radius\" [1]\nShape \"disk\"\n",
    )
    .unwrap();
    let document = read_document(&source).unwrap();
    let program = || PreviewProgram {
        directives: document.directives.clone(),
        origins: BTreeMap::new(),
        sources: BTreeMap::new(),
    };
    let store = dir.path().join("store");
    let mut cache = GeometryCache::default();
    let first = preview_from_program(
        program(),
        None,
        dir.path(),
        ParseOptions::default(),
        &mut cache,
    )
    .unwrap();
    write_preview(&first, &dir.path().join("first"), false).unwrap();
    write_preview_with_store(&first, &dir.path().join("first-store"), false, Some(&store)).unwrap();
    let stored_blobs = fs::read_dir(&store)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.metadata().unwrap().len(),
            )
        })
        .collect();
    let second = preview_from_program(
        program(),
        None,
        dir.path(),
        ParseOptions {
            stored_blobs,
            ..ParseOptions::default()
        },
        &mut cache,
    )
    .unwrap();
    assert!(second.blobs.is_empty());
    write_preview_with_store(&second, &dir.path().join("second"), false, Some(&store)).unwrap();
    assert_eq!(
        fs::read_dir(&store).unwrap().count(),
        first.manifest.blobs.len()
    );
    for (name, file) in &second.manifest.blobs {
        let store_blob = store.join(&file.sha256);
        assert_eq!(
            fs::read(dir.path().join("second").join(name)).unwrap(),
            fs::read(&store_blob).unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(dir.path().join("second").join(name))
                    .unwrap()
                    .ino(),
                fs::metadata(store_blob).unwrap().ino()
            );
        }
    }

    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\" \"float radius\" [2]\nShape \"disk\"\n",
    )
    .unwrap();
    let changed_document = read_document(&source).unwrap();
    let changed = preview_from_program(
        PreviewProgram {
            directives: changed_document.directives,
            origins: BTreeMap::new(),
            sources: BTreeMap::new(),
        },
        None,
        dir.path(),
        ParseOptions {
            stored_blobs: fs::read_dir(&store)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        entry.metadata().unwrap().len(),
                    )
                })
                .collect(),
            ..ParseOptions::default()
        },
        &mut cache,
    )
    .unwrap();
    assert_eq!(changed.blobs.len(), 1);
    let old_hashes: std::collections::BTreeSet<_> = first
        .manifest
        .blobs
        .values()
        .map(|file| file.sha256.as_str())
        .collect();
    let new_hashes: std::collections::BTreeSet<_> = changed
        .manifest
        .blobs
        .values()
        .map(|file| file.sha256.as_str())
        .collect();
    assert_eq!(new_hashes.difference(&old_hashes).count(), 1);
    write_preview_with_store(&changed, &dir.path().join("changed"), false, Some(&store)).unwrap();
    assert_eq!(
        fs::read_dir(&store).unwrap().count(),
        first.manifest.blobs.len() + 1
    );
}

#[test]
fn stored_image_asset_is_not_read_again_when_source_is_gone() {
    use sha2::Digest;

    let dir = tempfile::tempdir().unwrap();
    let asset = dir.path().join("image.dat");
    let bytes = b"image payload";
    fs::write(&asset, bytes).unwrap();
    let sha = format!("{:x}", sha2::Sha256::digest(bytes));
    let source = dir.path().join("scene.pbrt");
    fs::write(
        &source,
        format!(
            "WorldBegin\nTexture \"image\" \"spectrum\" \"imagemap\" \"string filename\" [{}]\n",
            serde_json::to_string(&asset.to_string_lossy()).unwrap()
        ),
    )
    .unwrap();
    let document = read_document(&source).unwrap();
    let program = || PreviewProgram {
        directives: document.directives.clone(),
        origins: BTreeMap::new(),
        sources: BTreeMap::new(),
    };
    let mut cache = GeometryCache::default();
    let options = ParseOptions {
        asset_sha256: [(asset.to_string_lossy().into_owned(), sha.clone())].into(),
        ..ParseOptions::default()
    };
    let first =
        preview_from_program(program(), None, dir.path(), options.clone(), &mut cache).unwrap();
    let store = dir.path().join("store");
    write_preview_with_store(&first, &dir.path().join("first"), false, Some(&store)).unwrap();
    fs::remove_file(&asset).unwrap();
    let second = preview_from_program(
        program(),
        None,
        dir.path(),
        ParseOptions {
            stored_blobs: [(sha.clone(), bytes.len() as u64)].into(),
            ..options
        },
        &mut cache,
    )
    .unwrap();
    assert!(second.blobs.is_empty());
    assert_eq!(second.manifest.blobs.values().next().unwrap().sha256, sha);
    let second_package = dir.path().join("second");
    write_preview_with_store(&second, &second_package, false, Some(&store)).unwrap();
    let packaged = read_preview_package(&second_package).unwrap();
    assert_eq!(packaged.blobs.values().next().unwrap(), bytes);
}
