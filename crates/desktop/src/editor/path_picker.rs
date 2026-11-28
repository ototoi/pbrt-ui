//! File dialogs with an explicit starting folder where the platform supports it.
use super::recent::{RecentEntry, RecentKind};
use gpui_kit::{Context, PathPromptOptions, Task};
use std::path::{Path, PathBuf};

pub(crate) fn recent_directory(entries: &[RecentEntry], kind: RecentKind, home: &Path) -> PathBuf {
    entries
        .iter()
        .find(|entry| entry.kind == kind)
        .and_then(|entry| entry.path.parent())
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.to_path_buf())
}

pub(crate) fn preferred_directory(
    current_project: Option<&Path>,
    recent: &[RecentEntry],
    home: &Path,
) -> PathBuf {
    current_project
        .and_then(Path::parent)
        .filter(|parent| parent.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| recent_directory(recent, RecentKind::Project, home))
}

pub(crate) fn pick_paths<T: 'static>(
    options: PathPromptOptions,
    // Only the Linux portal opens at a starting folder.
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))] initial_dir: PathBuf,
    cx: &mut Context<T>,
) -> Task<Result<Option<Vec<PathBuf>>, String>> {
    #[cfg(target_os = "linux")]
    {
        cx.spawn(async move |_, _| {
            use ashpd::desktop::file_chooser::OpenFileRequest;
            let request = OpenFileRequest::default()
                .modal(true)
                .title(if options.directories {
                    "Open Folder"
                } else {
                    "Open File"
                })
                .accept_label(options.prompt.as_ref().map(|prompt| prompt.as_str()))
                .multiple(options.multiple)
                .directory(options.directories)
                .current_folder(&initial_dir)
                .map_err(|error| format!("{error:?}"))?;
            let response = match request
                .send()
                .await
                .map_err(|error| format!("{error:?}"))?
                .response()
            {
                Ok(response) => response,
                // Cancel and close arrive as a portal response, as in gpui.
                Err(ashpd::Error::Response(_)) => return Ok(None),
                Err(error) => return Err(format!("{error:?}")),
            };
            Ok(Some(
                response
                    .uris()
                    .iter()
                    .filter_map(|uri| portal_file_uri_path(uri.as_str()))
                    .collect(),
            ))
        })
    }

    #[cfg(target_os = "macos")]
    {
        cx.spawn(async move |_, _| {
            use block2::RcBlock;
            use objc2::{MainThreadMarker, rc::Retained};
            use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSOpenPanel};
            use objc2_foundation::{NSString, NSURL};
            use std::cell::Cell;

            let marker = MainThreadMarker::new().expect("GPUI picker runs on the main thread");
            let panel = NSOpenPanel::openPanel(marker);
            panel.setCanChooseDirectories(options.directories);
            panel.setCanChooseFiles(options.files);
            panel.setAllowsMultipleSelection(options.multiple);
            panel.setCanCreateDirectories(true);
            panel.setResolvesAliases(false);
            let directory = NSURL::from_directory_path(&initial_dir);
            panel.setDirectoryURL(directory.as_deref());
            if let Some(prompt) = options.prompt {
                panel.setPrompt(Some(&NSString::from_str(prompt.as_str())));
            }

            let (sender, receiver) = futures_channel::oneshot::channel();
            let sender = Cell::new(Some(sender));
            let handler = RcBlock::new({
                let panel: Retained<NSOpenPanel> = panel.clone();
                move |response: NSModalResponse| {
                    let Some(sender) = sender.take() else { return };
                    let paths = (response == NSModalResponseOK).then(|| {
                        panel
                            .URLs()
                            .iter()
                            .filter(|url| url.isFileURL())
                            .filter_map(|url| url.to_file_path())
                            .collect()
                    });
                    let _ = sender.send(Ok(paths));
                }
            });
            panel.beginWithCompletionHandler(&handler);
            receiver.await.map_err(|error| format!("{error:?}"))?
        })
    }

    #[cfg(target_os = "windows")]
    {
        // The Windows common dialog remembers its last folder.
        let picker = cx.prompt_for_paths(options);
        cx.spawn(async move |_, _| match picker.await {
            Ok(result) => result.map_err(|error| format!("{error:?}")),
            Err(error) => Err(format!("{error:?}")),
        })
    }
}

#[cfg(target_os = "linux")]
fn portal_file_uri_path(uri: &str) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let uri_path = uri.strip_prefix("file://")?;
    let (uri_path, local_host) = if uri_path.starts_with('/') {
        (uri_path, false)
    } else {
        (uri_path.strip_prefix("localhost/")?, true)
    };
    let mut bytes = Vec::with_capacity(uri_path.len());
    if local_host {
        bytes.push(b'/');
    }
    let mut chars = uri_path.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let high = hex(chars.next()?)?;
            let low = hex(chars.next()?)?;
            bytes.push((high << 4) | low);
        } else {
            bytes.push(byte);
        }
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(target_os = "linux")]
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn entry(kind: RecentKind, path: PathBuf) -> RecentEntry {
        RecentEntry {
            kind,
            path,
            title: String::new(),
            opened_at: String::new(),
        }
    }

    #[test]
    fn recent_directory_uses_the_latest_matching_existing_parent() {
        let root = std::env::temp_dir().join(format!(
            "pbrt-ui-picker-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project_dir = root.join("project");
        let pbrt_dir = root.join("pbrt");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::create_dir_all(&pbrt_dir).unwrap();
        let entries = [
            entry(RecentKind::Pbrt, pbrt_dir.join("scene.pbrt")),
            entry(RecentKind::Project, project_dir.join("project.pbrtui")),
        ];
        assert_eq!(
            recent_directory(&entries, RecentKind::Pbrt, Path::new("/home/test")),
            pbrt_dir
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn no_recent_entry_or_missing_parent_uses_home() {
        let home = Path::new("/home/test");
        assert_eq!(recent_directory(&[], RecentKind::Project, home), home);
        let entries = [entry(
            RecentKind::Project,
            PathBuf::from("/missing/path/project.pbrtui"),
        )];
        assert_eq!(recent_directory(&entries, RecentKind::Project, home), home);
    }

    #[test]
    fn current_project_directory_precedes_recent_project() {
        let root =
            std::env::temp_dir().join(format!("pbrt-ui-picker-project-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            preferred_directory(
                Some(&root.join("scene.pbrtui")),
                &[],
                Path::new("/home/test")
            ),
            root
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_file_uris_decode_paths_and_reject_non_local_files() {
        assert_eq!(
            portal_file_uri_path("file:///tmp/my%20scene.pbrt"),
            Some(PathBuf::from("/tmp/my scene.pbrt"))
        );
        assert_eq!(
            portal_file_uri_path("file://localhost/tmp/scene.pbrt"),
            Some(PathBuf::from("/tmp/scene.pbrt"))
        );
        assert_eq!(portal_file_uri_path("https://example.com/scene.pbrt"), None);
        assert_eq!(portal_file_uri_path("file://remote/tmp/scene.pbrt"), None);
    }
}
