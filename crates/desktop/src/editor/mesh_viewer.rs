//! The Mesh Viewer tab: a read-only 3D view of
//! one PLY asset with its counts, bounds and attributes. The geometry is
//! parsed off the GPUI thread and drawn by the tab's own renderer thread
//! Right-drag orbits, middle-drag pans and the wheel
//! dollies, as in the Scene panel. The first frame, shrunk to a thumbnail,
//! goes to the project cache's `Previews/` for later thumbnails.
use super::Session;
use crate::backend::Command;
use crate::mesh_view::{self, MeshStats, Progress, Request, Worker};
use gpui_kit::component::{Selectable as _, Sizable, button::Button, spinner::Spinner};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::Method;
use scene_parse::ply::PlyInfo;
use scene_view::{PreviewMode, ViewerCamera};
use serde_json::{Value, json};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

/// The cached preview's longest side.
const THUMBNAIL: u32 = 256;

pub struct MeshViewer {
    session: Entity<Session>,
    entry: Value,
    worker: Option<Worker>,
    stats: Option<MeshStats>,
    info: Option<PlyInfo>,
    message: String,
    /// The PLY load is still running: the "Loading…" note spins.
    loading: bool,
    camera: Option<ViewerCamera>,
    mode: PreviewMode,
    image: Option<Arc<RenderImage>>,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    /// The size the last request was for; a new viewport size re-renders.
    requested: [u32; 2],
    progress: Progress,
    /// A poll task is redrawing until the newest frame lands.
    polling: bool,
    last_mouse: Option<Point<Pixels>>,
    cached: bool,
    _release: Subscription,
}

impl MeshViewer {
    pub fn new(
        session: Entity<Session>,
        entry: Value,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let message = match &path {
            Some(path) => {
                let path = path.clone();
                cx.spawn(async move |this, cx| {
                    let loaded = cx
                        .background_executor()
                        .spawn(async move {
                            // Only the header for the statistics: assets may
                            // be gigabytes.
                            let info =
                                scene_parse::ply::ply_info_from_file(std::path::Path::new(&path))
                                    .ok();
                            mesh_view::load(&path).map(|geometry| (geometry, info))
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        match loaded {
                            Ok((geometry, info)) => {
                                this.stats = Some(mesh_view::stats(&geometry));
                                this.info = info;
                                this.worker = Some(Worker::start(geometry));
                                this.message.clear();
                            }
                            Err(error) => this.message = error,
                        }
                        this.loading = false;
                        cx.notify();
                    });
                })
                .detach();
                "Loading…".to_owned()
            }
            None => "The mesh file is not available (the project has no asset folder)".to_owned(),
        };
        // The last frame leaves the GPU atlas with the tab.
        let release = cx.on_release_in(window, |this: &mut Self, window, _| {
            if let Some(image) = this.image.take() {
                let _ = window.drop_image(image);
            }
        });
        Self {
            session,
            entry,
            worker: None,
            stats: None,
            info: None,
            message,
            loading: path.is_some(),
            camera: None,
            mode: PreviewMode::Solid,
            image: None,
            viewport: Rc::new(Cell::new(Bounds::default())),
            requested: [0, 0],
            progress: Progress::default(),
            polling: false,
            last_mouse: None,
            cached: false,
            _release: release,
        }
    }

    fn request(&mut self, size: [u32; 2], cx: &mut Context<Self>) {
        let (Some(worker), Some(camera)) = (&self.worker, self.camera) else {
            return;
        };
        self.requested = size;
        let generation = self.progress.next();
        let sent = worker.render(Request {
            generation,
            camera,
            size,
            mode: self.mode,
        });
        if !sent {
            // The worker stopped; its reason already
            // landed, so do not wait for a frame that never comes.
            self.progress.abandon();
            return;
        }
        self.poll(cx);
    }

    /// Redraw every frame until the newest request has landed, then stop:
    /// an idle tab wakes nothing.
    fn poll(&mut self, cx: &mut Context<Self>) {
        if self.polling {
            return;
        }
        self.polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let waiting = this.update(cx, |this, cx| {
                    cx.notify();
                    this.polling = this.progress.waiting();
                    this.polling
                });
                if !matches!(waiting, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Point the camera at the whole mesh (once it has loaded).
    fn frame(&mut self, cx: &mut Context<Self>) {
        if self.worker.is_none() {
            return;
        }
        let [width, height] = self.requested;
        let bounds = self
            .stats
            .as_ref()
            .and_then(|stats| stats.bounds)
            .map(|(min, max)| protocol::geometry::Bounds { min, max });
        self.camera = Some(ViewerCamera::frame(
            bounds.as_ref(),
            width.max(1) as f64 / height.max(1) as f64,
        ));
        self.request(self.requested, cx);
    }

    fn move_camera(
        &mut self,
        change: impl FnOnce(&mut ViewerCamera) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(camera) = &mut self.camera
            && change(camera).is_ok()
        {
            self.request(self.requested, cx);
        }
    }

    /// Put the first frame, shrunk to a thumbnail and encoded off the UI
    /// thread, in the project's cache (`Previews/`).
    fn cache(&mut self, rgba: Vec<u8>, width: u32, height: u32, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project_value();
        if self.cached || !project.is_object() {
            return;
        }
        self.cached = true;
        let item = self.entry["id"].clone();
        let session = self.session.clone();
        cx.spawn(async move |_, cx| {
            let encoded = cx
                .background_executor()
                .spawn(async move {
                    let image = image::RgbaImage::from_raw(width, height, rgba)?;
                    let scale = THUMBNAIL as f64 / width.max(height) as f64;
                    let image = if scale < 1. {
                        image::imageops::thumbnail(
                            &image,
                            ((width as f64 * scale) as u32).max(1),
                            ((height as f64 * scale) as u32).max(1),
                        )
                    } else {
                        image
                    };
                    let mut png = std::io::Cursor::new(Vec::new());
                    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
                    Some(base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        png.into_inner(),
                    ))
                })
                .await;
            let Some(data) = encoded else {
                return;
            };
            session.update(cx, |session, cx| {
                session.send(
                    Command::Project(
                        Method::ProjectPutCache,
                        json!({"project":project,"item":item,"key":"mesh-preview.png",
                               "kind":"preview","media_type":"image/png","data_base64":data}),
                    ),
                    cx,
                );
            });
        })
        .detach();
    }

    fn info_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(stats) = &self.stats {
            lines.push(format!(
                "{} vertices · {} triangles",
                stats.vertices, stats.triangles
            ));
            if let Some((min, max)) = stats.bounds {
                let fmt = |v: [f64; 3]| format!("{:.3} {:.3} {:.3}", v[0], v[1], v[2]);
                lines.push(format!("bounds {} … {}", fmt(min), fmt(max)));
            }
        }
        if let Some(info) = &self.info {
            let yes = |flag: bool| if flag { "yes" } else { "no" };
            lines.push(format!(
                "normals {} · UVs {} · {}",
                yes(info.normals),
                yes(info.uvs),
                info.format
            ));
        }
        lines
    }
}

impl Render for MeshViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Land the newest frame; the old image leaves the GPU atlas.
        let mut landed = None;
        if let Some(worker) = &self.worker {
            while let Ok(frame) = worker.frames.try_recv() {
                landed = Some(frame);
            }
        }
        if let Some((generation, frame)) = landed {
            self.progress.land(generation);
            match frame {
                Ok(mut frame) => {
                    self.message.clear();
                    self.cache(frame.rgba.clone(), frame.width, frame.height, cx);
                    for pixel in frame.rgba.as_chunks_mut::<4>().0 {
                        pixel.swap(0, 2);
                    }
                    if let Some(buffer) =
                        image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
                    {
                        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                        if let Some(old) = self.image.replace(image) {
                            let _ = window.drop_image(old);
                        }
                    }
                }
                Err(error) => self.message = error,
            }
        }
        // Frame on the first known size; re-render when the size changes.
        let bounds = self.viewport.get();
        let size = [
            f32::from(bounds.size.width).max(1.) as u32,
            f32::from(bounds.size.height).max(1.) as u32,
        ];
        if self.worker.is_some() && size != self.requested && size[0] > 1 && size[1] > 1 {
            self.requested = size;
            if self.camera.is_none() {
                self.frame(cx);
            } else {
                self.request(size, cx);
            }
        }
        let viewport = self.viewport.clone();
        let mode_button = |label: &'static str, mode: PreviewMode, current: PreviewMode| {
            Button::new(SharedString::from(format!("mesh-mode-{label}")))
                .label(label)
                .xsmall()
                .when(mode == current, |button| button.selected(true))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.mode = mode;
                    this.request(this.requested, cx);
                    cx.notify();
                }))
        };
        let current = self.mode;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x17191e))
            .text_color(rgb(0xd4d7dd))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .bg(rgb(0x25272d))
                    .child(mode_button("Solid", PreviewMode::Solid, current))
                    .child(mode_button("Normal", PreviewMode::Normal, current))
                    .child(mode_button("Wireframe", PreviewMode::Wireframe, current))
                    .child(
                        Button::new("mesh-frame")
                            .label("Frame")
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.frame(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9098a6))
                            .child("Read-only · RMB orbit · MMB pan · scroll zoom"),
                    ),
            )
            .child(
                div()
                    .id("mesh-viewport")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                // A new size is known only after layout:
                                // draw again so render can request it.
                                // refresh() is ignored mid-draw.
                                if viewport.replace(bounds).size != bounds.size {
                                    window.request_animation_frame();
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(self.image.clone().map(|image| {
                        img(image)
                            .absolute()
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.last_mouse = Some(event.position)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(|this, event: &MouseDownEvent, _, _| {
                            this.last_mouse = Some(event.position)
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        let button = event.pressed_button;
                        if !matches!(button, Some(MouseButton::Right | MouseButton::Middle)) {
                            this.last_mouse = None;
                            return;
                        }
                        if let Some(last) = this.last_mouse.replace(event.position) {
                            let dx = f64::from(f32::from(event.position.x - last.x));
                            let dy = f64::from(f32::from(event.position.y - last.y));
                            // The Scene panel's factors.
                            if button == Some(MouseButton::Right) {
                                this.move_camera(
                                    |camera| camera.orbit(-dx * 0.008, -dy * 0.008),
                                    cx,
                                );
                            } else {
                                this.move_camera(|camera| camera.pan(-dx * 0.002, dy * 0.002), cx);
                            }
                            cx.notify();
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        let delta = match event.delta {
                            ScrollDelta::Pixels(p) => f64::from(f32::from(p.y)) * 0.005,
                            ScrollDelta::Lines(p) => f64::from(p.y) * 0.12,
                        };
                        // Wheel up moves closer, matching the Scene viewport.
                        this.move_camera(|camera| camera.dolly(delta), cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .flex()
                    .flex_col()
                    .text_xs()
                    .bg(rgb(0x25272d))
                    .text_color(rgb(0x9098a6))
                    .child(
                        self.entry["location"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    )
                    .children(self.info_lines())
                    .when(self.loading || !self.message.is_empty(), |this| {
                        // Non-blocking status: a small spinner beside the
                        // "Loading…" note while the PLY load runs.
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(self.loading, |row| row.child(Spinner::new().xsmall()))
                                .when(!self.message.is_empty(), |row| {
                                    row.child(
                                        div().text_color(rgb(0xd99a74)).child(self.message.clone()),
                                    )
                                }),
                        )
                    }),
            )
    }
}
