//! The regenerable project cache under `Library/Cache/`:
//! an index plus per-item files, written atomically by the lock holder, with
//! a size limit that evicts by last use. It is never project content: saving
//! does not journal it, does not trip external-change detection, and Save As
//! does not copy it.
use project::*;
use serde_json::json;
use std::fs;

fn temp_project(name: &str) -> (tempfile::TempDir, ProjectDocument) {
    let temporary = tempfile::tempdir().unwrap();
    let document = ProjectDocument::new(name);
    let path = temporary.path().join("project");
    FolderStore::create(&path, document.id, &document.title, Revision::initial()).unwrap();
    (temporary, document)
}

fn root(temporary: &tempfile::TempDir) -> std::path::PathBuf {
    temporary.path().join("project/Library/Cache")
}

#[test]
fn concurrent_cache_api_calls_preserve_every_index_entry() {
    let (temporary, _) = temp_project("Concurrent cache");
    let cache_root = root(&temporary);
    ensure_cache(&cache_root).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(9));
    let threads = (0..8)
        .map(|index| {
            let barrier = barrier.clone();
            let root = if index % 2 == 0 {
                cache_root.clone()
            } else {
                cache_root.parent().unwrap().join("Cache/../Cache")
            };
            std::thread::spawn(move || {
                barrier.wait();
                let item = format!("item-{index}");
                cache_put(
                    &root,
                    &item,
                    "preview",
                    "preview",
                    "image/png",
                    &[index],
                    DEFAULT_CACHE_LIMIT,
                )
                .unwrap();
                let _ = cache_get(&root, &item, "preview").unwrap();
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    for index in 0..8 {
        assert!(
            cache_get(&cache_root, &format!("item-{index}"), "preview")
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn cache_round_trips_and_recreates_a_missing_library() {
    let (temporary, _) = temp_project("Cache");
    let project = temporary.path().join("project");
    drop(FolderStore::open(&project).unwrap());
    // Disposable cache: removing Library must not break open.
    fs::remove_dir_all(temporary.path().join("project/Library")).unwrap();
    let store = FolderStore::open(&project)
        .map(|_| ())
        .map_err(|e| e.to_string());
    assert!(
        store.is_ok(),
        "open must recreate a missing Library: {store:?}"
    );
    let cache_root = root(&temporary);
    assert!(cache_root.join("Thumbnails").is_dir());
    assert!(cache_root.join("Previews").is_dir());
    let path = cache_put(
        &cache_root,
        "6868d5ba-99e5-4b94-9c38-64d7ab1ba18c",
        "sha256-abc-size-64",
        "thumbnail",
        "image/png",
        b"png bytes",
        DEFAULT_CACHE_LIMIT,
    )
    .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"png bytes");
    let hit = cache_get(
        &cache_root,
        "6868d5ba-99e5-4b94-9c38-64d7ab1ba18c",
        "sha256-abc-size-64",
    )
    .unwrap()
    .expect("cache hit");
    assert_eq!(hit.media_type, "image/png");
    assert_eq!(fs::read(&hit.path).unwrap(), b"png bytes");
    assert!(
        cache_get(&cache_root, "6868d5ba-99e5-4b94-9c38-64d7ab1ba18c", "other")
            .unwrap()
            .is_none(),
    );
    match cache_put(
        &cache_root,
        "x",
        "y",
        "framed",
        "image/png",
        b"",
        DEFAULT_CACHE_LIMIT,
    ) {
        Err(ProjectError::Invalid(_)) => {}
        other => panic!("unknown cache kind must be invalid: {other:?}"),
    }
}

#[test]
fn cache_writes_do_not_trip_external_change_detection_on_save() {
    let (temporary, document) = temp_project("Cache save");
    let mut store = FolderStore::open(&temporary.path().join("project")).unwrap();
    cache_put(
        &root(&temporary),
        "6868d5ba-99e5-4b94-9c38-64d7ab1ba18c",
        "key",
        "preview",
        "image/png",
        b"preview bytes",
        DEFAULT_CACHE_LIMIT,
    )
    .unwrap();
    let session = ProjectSession::new(document).unwrap();
    for round in 0..3 {
        let error = store.save(&session.snapshot());
        assert!(
            error.is_ok(),
            "cache writes are not project content (round {round}): {error:?}"
        );
        cache_put(
            &root(&temporary),
            "6868d5ba-99e5-4b94-9c38-64d7ab1ba18c",
            format!("key-{round}").as_str(),
            "preview",
            "image/png",
            b"preview bytes",
            DEFAULT_CACHE_LIMIT,
        )
        .unwrap();
    }
    assert!(
        root(&temporary)
            .join("Previews/6868d5ba-99e5-4b94-9c38-64d7ab1ba18c/key")
            .is_file()
    );
}

#[test]
fn cache_evicts_the_least_recently_used_over_the_limit() {
    let (temporary, _) = temp_project("Cache limit");
    let cache_root = root(&temporary);
    for (item, bytes) in [("aaaa", b"0123456789"), ("bbbb", b"0123456789")] {
        cache_put(&cache_root, item, "k", "thumbnail", "image/png", bytes, 25).unwrap();
    }
    let limit = 25;
    // Refresh aaaa so bbbb becomes the least recently used entry.
    cache_get(&cache_root, "aaaa", "k").unwrap().unwrap();
    cache_put(
        &cache_root,
        "cccc",
        "k",
        "thumbnail",
        "image/png",
        b"0123456789",
        limit,
    )
    .unwrap();
    assert!(cache_get(&cache_root, "aaaa", "k").unwrap().is_some());
    assert!(
        cache_get(&cache_root, "bbbb", "k").unwrap().is_none(),
        "the least recently used entry must be evicted"
    );
    assert!(cache_get(&cache_root, "cccc", "k").unwrap().is_some());
}

#[test]
fn cache_rejects_unsafe_identifiers_and_kinds() {
    let (temporary, _) = temp_project("Cache names");
    let cache_root = root(&temporary);
    for item in ["../escape", "a/b"] {
        assert!(
            cache_put(&cache_root, item, "k", "thumbnail", "image/png", b"x", 16).is_err(),
            "item {item:?} must be rejected"
        );
    }
    for key in ["../escape", "x/y", ""] {
        assert!(
            cache_put(&cache_root, "aaaa", key, "thumbnail", "image/png", b"x", 16).is_err(),
            "key {key:?} must be rejected"
        );
    }
    assert!(cache_get(&cache_root, "../escape", "k").is_err());
}

#[test]
fn a_broken_or_stale_index_is_regenerated() {
    let (temporary, _) = temp_project("Cache index");
    let cache_root = root(&temporary);
    fs::write(cache_root.join("cache.json"), b"{not json").unwrap();
    assert!(cache_get(&cache_root, "aaaa", "k").unwrap().is_none());
    cache_put(
        &cache_root,
        "aaaa",
        "k",
        "thumbnail",
        "image/png",
        b"png",
        16,
    )
    .unwrap();
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_root.join("cache.json")).unwrap()).unwrap();
    assert_eq!(index["format"], "pbrt-ui.cache/1");
    assert!(cache_get(&cache_root, "aaaa", "k").unwrap().is_some());
    // A file the index does not know is not a hit.
    fs::create_dir_all(cache_root.join("Thumbnails/zzzz")).unwrap();
    fs::write(cache_root.join("Thumbnails/zzzz/k"), b"stray").unwrap();
    assert!(cache_get(&cache_root, "zzzz", "k").unwrap().is_none());
    // An index entry whose file is gone heals to a miss.
    fs::remove_file(cache_root.join("Thumbnails/aaaa/k")).unwrap();
    assert!(cache_get(&cache_root, "aaaa", "k").unwrap().is_none());
}

#[test]
fn the_store_reports_its_cache_root() {
    let (temporary, _) = temp_project("Cache root");
    let store = FolderStore::open(&temporary.path().join("project")).unwrap();
    // The store canonicalizes; macOS temp dirs sit behind the /var symlink.
    assert_eq!(
        store.cache_root(),
        protocol::path::canonicalize(temporary.path())
            .unwrap()
            .join("project/Library/Cache")
    );
}

#[test]
fn a_single_entry_over_the_limit_is_kept() {
    let (temporary, _) = temp_project("Cache single");
    let cache_root = root(&temporary);
    let path = cache_put(
        &cache_root,
        "aaaa",
        "k",
        "thumbnail",
        "image/png",
        b"0123456789012345678901234567890",
        8,
    )
    .unwrap();
    assert!(path.is_file(), "the only entry is never evicted");
    assert!(cache_get(&cache_root, "aaaa", "k").unwrap().is_some());
}

#[test]
fn a_doctored_index_cannot_point_eviction_outside_the_cache() {
    let (temporary, _) = temp_project("Cache guard");
    let cache_root = root(&temporary);
    cache_put(
        &cache_root,
        "aaaa",
        "k",
        "thumbnail",
        "image/png",
        b"tiny",
        8,
    )
    .unwrap();
    let outside = temporary.path().join("outside-secret.txt");
    fs::write(&outside, b"do not touch").unwrap();
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(cache_root.join("cache.json")).unwrap()).unwrap();
    index["entries"]["../../../../outside-secret"] = json!({
        "kind": "thumbnail",
        "media_type": "image/png",
        "size": 1_000_000,
        "last_used": 0
    });
    fs::write(
        cache_root.join("cache.json"),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    // A put over the limit evicts the doctored entry without deleting anything.
    cache_put(
        &cache_root,
        "bbbb",
        "k",
        "thumbnail",
        "image/png",
        b"0123456789",
        10,
    )
    .unwrap();
    assert!(outside.is_file(), "the outside file must survive eviction");
    assert!(cache_get(&cache_root, "bbbb", "k").unwrap().is_some());
}
