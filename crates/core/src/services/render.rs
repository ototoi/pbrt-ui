//! Native-project ray render and progressive image conversion.
use super::*;
pub(super) fn execute(
    work: &ServiceTask,
    directory: &PathBuf,
    artifacts: &mut Vec<Artifact>,
    progress: std::sync::mpsc::Sender<(protocol::JobId, Artifact)>,
) -> Result<Value, ErrorPayload> {
    match &work.work {
        Work::ProjectRender {
            root,
            settings,
            view,
            ..
        } => {
            let project = if let Work::ProjectRender { project, .. } = &work.work {
                Some(project.clone())
            } else {
                None
            };
            let ipr = matches!(&work.work, Work::ProjectRender { ipr: true, .. });
            let mut metadata = json!({"settings":settings,"camera_source":if view.is_some(){"view"}else{"scene"},"view":view});
            if let Some(project) = &project {
                metadata["project"] = json!(project);
                let manifest = Path::new(root).parent().unwrap().join("export.json");
                metadata["export"] =
                    serde_json::from_slice(&fs::read(&manifest).map_err(fail)?).map_err(fail)?;
                if !ipr {
                    artifacts.push(artifact(
                        &manifest,
                        "application/json",
                        json!({"kind":"project_export_manifest"}),
                    )?);
                }
            }
            let renderer =
                work.config.renderer.as_ref().ok_or_else(|| {
                    ErrorPayload::new(ErrorCode::Unsupported, "renderer unavailable")
                })?;
            let output = directory.join("render.exr");
            let log = directory.join("render.log");
            let mut command = Command::new(renderer);
            let root = if let Some(view) = view {
                write_view_scene(root, view, directory)?
            } else {
                PathBuf::from(root)
            };
            if matches!(&work.work, Work::ProjectRender { .. }) {
                metadata["render_input_sha256"] =
                    json!(artifact(&root, "text/plain", Value::Null)?.sha256);
            }
            command.arg("--outfile").arg(&output);
            for argument in render_arguments(settings, ipr) {
                command.arg(argument);
            }
            let listener = if work.config.capture.is_some() {
                let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                    .map_err(fail)?;
                command
                    .arg("--display-server")
                    .arg(listener.local_addr().map_err(fail)?.to_string());
                Some(listener)
            } else {
                None
            };
            command.arg(&root);
            let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let monitor = listener.zip(work.config.capture.clone()).map(|(listener, converter)| {
                    let stopped = stopped.clone();
                    let task = work.task.clone();
                    let directory = directory.clone();
                    std::thread::spawn(move || {
                        let mut count = 0;
 // IPR jobs are short; a final render stretches its interval
 // (`publish_interval`) and keeps only the latest snapshot.
                        let interval = |elapsed| if ipr { std::time::Duration::from_millis(500) } else { crate::render_progress::publish_interval(elapsed) };
                        let result = crate::render_progress::receive(listener, stopped, || task.is_cancelled(), interval, |image| {
                            let raw = directory.join("progress.rgb32f");
                            if image.write_snapshot(&raw).is_err() { return; }
                            let preview = directory.join(if ipr {
                                format!("ipr-frame-{count:05}.png")
                            } else {
                                format!("progress-{count:03}.png")
                            });
                            let mut command = Command::new(&converter);
                            command.arg("--convert-f32").arg(&raw).arg(&preview);
                            if run_process(command, &task, &directory.join("progress-convert.log")).is_ok() {
                                let received = image.received_fraction();
                                let metadata = if ipr {
                                    json!({"kind":"ipr_frame","partial":true,"tone_map":"reinhard_srgb","project":project,"generation":count,"received":received})
                                } else {
                                    json!({"kind":"render_preview","partial":true,"tone_map":"reinhard_srgb","project":project,"received":received})
                                };
                                if let Ok(artifact) = artifact(&preview, "image/png", metadata) {
                                    let _ = progress.send((task.job_id, artifact));
                                }
                                count += 1;
                            }
                        });
                        let _ = fs::remove_file(directory.join("progress.rgb32f"));
                        if let Err(error) = result { let _ = fs::write(directory.join("progress-error.log"), error.to_string()); }
                    })
                });
            let process = run_process(command, &work.task, &log);
            stopped.store(true, std::sync::atomic::Ordering::Release);
            if let Some(monitor) = monitor {
                let _ = monitor.join();
            }
            if !ipr {
                artifacts.push(artifact(
                    &log,
                    "text/plain",
                    json!({"kind":"renderer_log"}),
                )?);
            }
            process?;
            let mut signature = [0; 4];
            fs::File::open(&output)
                .map_err(fail)?
                .read_exact(&mut signature)
                .map_err(fail)?;
            if signature != [0x76, 0x2f, 0x31, 0x01] {
                return Err(ErrorPayload::new(
                    ErrorCode::OutputInvalid,
                    "renderer output is not an EXR image",
                ));
            }
            if ipr {
                // Progressive results are ephemeral frames, not artifacts.
                return Ok(json!({"ipr":true}));
            }
            let result = artifact(&output, "image/x-exr", metadata)?;
            let id = result.artifact_id;
            artifacts.push(result);
            if let Some(converter) = &work.config.capture {
                let preview = directory.join("render.png");
                let mut command = Command::new(converter);
                command.arg("--convert").arg(&output).arg(&preview);
                run_process(command, &work.task, &directory.join("convert.log")).map_err(
                    |error| {
                        ErrorPayload::new(
                            ErrorCode::OutputInvalid,
                            format!("EXR validation failed: {}", error.message),
                        )
                    },
                )?;
                artifacts.push(artifact(&preview,"image/png",json!({"kind":"render_preview","tone_map":"reinhard_srgb","camera_source":if view.is_some(){"view"}else{"scene"}}))?);
            }
            Ok(json!({"artifact_id":id}))
        }
        _ => unreachable!("render dispatch"),
    }
}

fn render_arguments(settings: &protocol::jobs::RenderSettings, ipr: bool) -> Vec<String> {
    let mut arguments = Vec::new();
    if let Some(samples) = settings.samples_per_pixel {
        arguments.extend(["--spp".to_owned(), samples.to_string()]);
    }
    if let Some(threads) = settings.threads {
        arguments.extend(["--nthreads".to_owned(), threads.to_string()]);
    }
    if let Some(depth) = settings.max_depth {
        arguments.extend(["--maxdepth".to_owned(), depth.to_string()]);
    }
    if settings.gpu && !ipr {
        arguments.push("--gpu".to_owned());
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::render_arguments;
    use protocol::jobs::RenderSettings;

    #[test]
    fn gpu_argument_is_used_only_for_final_render_jobs() {
        let settings = RenderSettings {
            gpu: true,
            ..RenderSettings::default()
        };
        assert_eq!(render_arguments(&settings, false), ["--gpu"]);
        assert!(render_arguments(&settings, true).is_empty());
        let settings = RenderSettings::default();
        assert!(render_arguments(&settings, false).is_empty());
    }

    #[test]
    fn samples_argument_is_used_only_when_set() {
        let settings = RenderSettings {
            samples_per_pixel: Some(32),
            ..RenderSettings::default()
        };
        assert_eq!(render_arguments(&settings, false), ["--spp", "32"]);
        assert!(render_arguments(&RenderSettings::default(), false).is_empty());
    }
}
