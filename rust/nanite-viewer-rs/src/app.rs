//! The application shell: winit event loop, wgpu surface, egui overlay, and the frame loop.
//!
//! This is the Rust mirror of `VulkanExampleBase` plus `VulkanExample::render()` in
//! `examples/pbrtexture/pbrtexture.cpp`. The frame order is copied from
//! `VulkanExampleBase::nextFrame()` on purpose — the C++ regression tests depend on *when* a
//! reload happens relative to a captured frame, and on the last frame being written to disk
//! before it is presented:
//!
//! ```text
//! nextFrame():
//!   if viewUpdated { viewUpdated = false; viewChanged(); }
//!   if capture.active { capture.pending = framesRendered + 1 >= capture.frames }
//!   render()            // pbrtexture: dialog, reload, draw(), stats
//!   if capture.active { capture.framesRendered++ }
//!   frameCounter++
//!   camera.update(frameTimer)          // AFTER render
//!   if camera.moving() { viewUpdated = true }
//!   fps bookkeeping (SetWindowText while the overlay is off)
//!   updateOverlay()
//! ```
//!
//! # Deferred input
//!
//! The C++ viewer opens a *modal* Win32 dialog and calls `viewAll()` straight from `keyPressed`.
//! Neither is possible from inside a winit `WindowEvent` here: the dialog pumps window messages,
//! which re-enters the event loop while `about_to_wait` is on the stack, and egui owns the input
//! for the rest of the frame. So every input handler only *requests* work and `next_frame()`
//! applies it at the top of the frame — the same place pbrtexture's `render()` calls
//! `handlePendingModelDialog()` / `handlePendingCommandLineReload()` (`pbrtexture.cpp:4376`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result};
use glam::{Mat4, Vec3};
use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::camera::Camera;
use crate::capture::CaptureTarget;
use crate::load::{self, LoadedModel, ReloadRequest};
use crate::renderer::{FrameSettings, RasterStats, Renderer, THRESHOLD_INT_DIV};
use crate::scene::Scene;

/// `pbrtexture.cpp:351` — `camera.rotationSpeed = 0.25f`.
const ROTATION_SPEED: f32 = 0.25;
/// `handleMouseMove()`: the hard-coded `0.005f` for the right and middle mouse buttons.
const TRANSLATE_SPEED: f32 = 0.005;
/// `vulkanexamplebase.h:161-162`.
const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;
/// `pbrtexture.cpp:346`, the window title.
const WINDOW_TITLE: &str = "Vulkanite: A dynamic LOD rendering pipeline";

/// How the model is obtained — the mirror of `NaniteMesh::BlobMode`.
///
/// `load::SourceMode` is the real switch; this is the CLI-facing spelling of it. There is no
/// `JsonOnly`: the Rust pipeline has no `nanite_info.json` cache, so `--nanite-json` degrades to
/// `Auto` with a warning instead of pretending it does something.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceMode {
    /// Use `<model>.fcc` when it exists, otherwise bake the glTF in-process.
    #[default]
    Auto,
    /// Fail when no `.fcc` exists (`--nanite-blob`).
    Require,
    /// Always rebuild (`--nanite-rebuild`).
    Rebuild,
}

/// Command line, mirroring the pbrtexture flags one for one (`pbrtexture.cpp:316-350`).
#[derive(Clone, Debug)]
pub struct Args {
    /// `--model <name>`: a bare name resolved under `assets/models`, a path, or a `.fcc` blob.
    pub model: String,
    pub mode: SourceMode,
    /// `--reload` / `--reload2`, sorted by the frame they fire on.
    pub reloads: Vec<ReloadRequest>,
    /// `--visualize <0..3>`.
    pub visualize: i32,
    /// `--threshold <0..1000>`, the integer form the C++ slider edits.
    pub threshold: i32,
    /// `--raster-stats`.
    pub raster_stats: bool,
    /// `--capture <file.png>`.
    pub capture: Option<PathBuf>,
    /// `--captureframes <N>`, default 10.
    pub capture_frames: u64,
    pub width: u32,
    pub height: u32,
    /// `--log <level>`: `off|error|warn|info|debug|trace`.
    pub log: String,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            // `pbrtexture.cpp:317` defaults to `dragon`, but `dragon` has no `.fcc` and the Rust
            // builder is the only way to get one here; `bunny` is the model every C++ test uses.
            model: "bunny".to_string(),
            mode: SourceMode::default(),
            reloads: Vec::new(),
            visualize: 0,
            threshold: 500,
            raster_stats: false,
            capture: None,
            capture_frames: 10,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            log: "info".to_string(),
        }
    }
}

impl Args {
    /// Parse `argv[1..]`. Written by hand rather than with `clap` because the C++ parser is
    /// positional-tolerant (`--model bunny` *and* a bare `bunny` both work) and a `clap` derive
    /// would change what the existing test command lines mean.
    pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> Result<Self> {
        let mut args = Args::default();
        // Collected up front: `next` below takes a concrete `Peekable<vec::IntoIter<String>>`, and
        // keeping the generic iterator would force the closure to be generic over `I::IntoIter`.
        let mut items = argv
            .into_iter()
            .collect::<Vec<String>>()
            .into_iter()
            .peekable();
        let mut model_given = false;
        // The two `--reload*` pairs are collected here and paired up below, because the value
        // flag (`--reloadframe`) comes after the file.
        let mut pending: Vec<(String, Option<u64>)> = Vec::new();

        while let Some(arg) = items.next() {
            let next = |items: &mut std::iter::Peekable<std::vec::IntoIter<String>>| -> Result<String> {
                items.next().ok_or_else(|| anyhow::anyhow!("{arg} needs a value"))
            };
            match arg.as_str() {
                "--model" => {
                    args.model = next(&mut items)?;
                    model_given = true;
                }
                "--nanite-blob" => args.mode = SourceMode::Require,
                "--nanite-json" => {
                    args.mode = SourceMode::Auto;
                    warn!("--nanite-json has no meaning in the Rust viewer: there is no JSON cache, only .fcc");
                }
                "--nanite-rebuild" => args.mode = SourceMode::Rebuild,
                "--reload" => pending.push((next(&mut items)?, None)),
                "--reloadframe" => {
                    let frame = parse_int(&next(&mut items)?, 10);
                    let slot = pending
                        .last_mut()
                        .ok_or_else(|| anyhow::anyhow!("--reloadframe must follow --reload <file>"))?;
                    slot.1 = Some(frame.max(1) as u64);
                }
                "--reload2" => pending.push((next(&mut items)?, None)),
                "--reload2frame" => {
                    let frame = parse_int(&next(&mut items)?, 20);
                    let slot = pending
                        .last_mut()
                        .ok_or_else(|| anyhow::anyhow!("--reload2frame must follow --reload2 <file>"))?;
                    slot.1 = Some(frame.max(1) as u64);
                }
                "--visualize" => args.visualize = parse_int(&next(&mut items)?, 0).clamp(0, 3) as i32,
                "--threshold" => {
                    args.threshold = parse_int(&next(&mut items)?, 500).clamp(0, 1000) as i32;
                }
                "--raster-stats" => args.raster_stats = true,
                "--capture" => args.capture = Some(PathBuf::from(next(&mut items)?)),
                "--captureframes" => {
                    args.capture_frames = parse_int(&next(&mut items)?, 10).max(1) as u64;
                }
                "--width" => {
                    args.width = parse_int(&next(&mut items)?, i64::from(DEFAULT_WIDTH)).max(1) as u32;
                }
                "--height" => {
                    args.height = parse_int(&next(&mut items)?, i64::from(DEFAULT_HEIGHT)).max(1) as u32;
                }
                "--log" => args.log = next(&mut items)?,
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other if !other.starts_with('-') && !model_given => {
                    args.model = other.to_string();
                    model_given = true;
                }
                other => anyhow::bail!("unknown option {other}"),
            }
        }

        for (model, frame) in pending {
            args.reloads.push(ReloadRequest::new(model, frame.unwrap_or(10)));
        }
        args.reloads.sort_by_key(|reload| reload.frame);
        Ok(args)
    }
}

fn print_usage() {
    println!(
        "nanite-viewer-rs - {WINDOW_TITLE}\n\
         \n\
         usage: nanite-viewer-rs [options] [model]\n\
         \n\
         --model <name>        model name, path, or .fcc blob (default: bunny)\n\
         --nanite-blob         require an existing .fcc\n\
         --nanite-rebuild      always re-bake the glTF\n\
         --reload <file>       load another model at runtime\n\
         --reloadframe <N>     frame the --reload fires on (default 10)\n\
         --reload2 <file>      a second reload\n\
         --reload2frame <N>    frame the --reload2 fires on (default 20)\n\
         --visualize <0..3>    cluster visualization mode\n\
         --threshold <0..1000> LOD selection threshold\n\
         --raster-stats        print the raster counters while capturing\n\
         --capture <file.png>  capture frames to a PNG\n\
         --captureframes <N>   number of frames to capture (default 10)\n\
         --width <N> / --height <N>\n\
         --log <level>         off|error|warn|info|debug|trace"
    );
}

/// `base/CommandLineParser.hpp::getValueAsInt`: the default is used only when *nothing* parsed,
/// so `0` and negative values survive. Do not add a `> 0` guard — that is what used to make
/// `--threshold 0` impossible.
fn parse_int(value: &str, default: i64) -> i64 {
    value.trim().parse::<i64>().unwrap_or(default)
}

/// WASD state, the mirror of `camera.keys` in the base framework.
#[derive(Clone, Copy, Debug, Default)]
struct Keys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

/// `handleMouseMove()`'s button state.
#[derive(Clone, Copy, Debug, Default)]
struct MouseButtons {
    left: bool,
    right: bool,
    middle: bool,
}

pub struct App {
    args: Args,
    /// `assets/models`, resolved once at start-up. The file dialog starts here, the model name is
    /// resolved against it, and the env cube is found by walking back up to `assets`.
    model_dir: PathBuf,

    window: Option<Arc<Window>>,
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    /// Kept because `Surface::get_capabilities` needs it, and a resize has to re-run
    /// [`pick_surface_config`] rather than reuse the old format.
    adapter: Option<wgpu::Adapter>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    config: Option<wgpu::SurfaceConfiguration>,
    adapter_name: String,
    renderer: Option<Renderer>,
    /// The counters of the last rendered frame. `Renderer::render` *returns* them, and the overlay
    /// is drawn after that call, so they have to be parked here - `Renderer` keeps no copy itself.
    last_stats: RasterStats,

    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,

    camera: Camera,
    settings: FrameSettings,
    /// The integer the Threshold slider edits; `settings.threshold` is `threshold * 1e-6`.
    threshold_int: i32,
    /// The "LOD level" slider. Inert in C++ as well — `vis_clusters_level` is only read from
    /// commented-out code (`pbrtexture.cpp:923`) — so it is kept for parity and nothing else.
    lod_level: i32,
    model: Option<LoadedModel>,
    scene: Option<Scene>,

    frames_rendered: u64,
    frame_counter: u32,
    last_fps: f32,
    frame_ms: f64,
    fps_timer: f64,
    t_prev_end: Instant,

    view_updated: bool,
    keys: Keys,
    /// The previous cursor position, `VulkanExampleBase::mousePos`. `None` means "no previous",
    /// which is what a button press sets so a drag never jumps across a gap.
    mouse_pos: Option<(f64, f64)>,
    mouse_buttons: MouseButtons,
    open_dialog_requested: bool,
    view_all_requested: bool,
    reset_camera_requested: bool,

    capture_target: Option<CaptureTarget>,
    capture_done: bool,
}

impl App {
    pub fn new(args: Args) -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        let model_dir = load::find_model_dir(&cwd).unwrap_or_else(|| PathBuf::from(load::MODEL_DIR));

        let mut settings = FrameSettings::default();
        settings.threshold = args.threshold as f32 * THRESHOLD_INT_DIV;
        settings.visualize = args.visualize;

        Self {
            args,
            model_dir,
            window: None,
            instance: wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()),
            surface: None,
            adapter: None,
            device: None,
            queue: None,
            config: None,
            adapter_name: String::new(),
            renderer: None,
            last_stats: RasterStats::default(),
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            camera: Camera::default(),
            settings,
            threshold_int: 500,
            lod_level: 0,
            model: None,
            scene: None,
            frames_rendered: 0,
            frame_counter: 0,
            last_fps: 0.0,
            frame_ms: 0.0,
            fps_timer: 0.0,
            t_prev_end: Instant::now(),
            view_updated: false,
            keys: Keys::default(),
            mouse_pos: None,
            mouse_buttons: MouseButtons::default(),
            open_dialog_requested: false,
            view_all_requested: false,
            reset_camera_requested: false,
            capture_target: None,
            capture_done: false,
        }
    }

    // ---------------------------------------------------------------- GPU start-up

    fn resume(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let attributes = WindowAttributes::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size(PhysicalSize::new(self.args.width, self.args.height))
            .with_min_inner_size(PhysicalSize::new(320u32, 240u32));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .context("could not create the window")?,
        );
        self.args.width = window.inner_size().width.max(1);
        self.args.height = window.inner_size().height.max(1);

        let surface = self
            .instance
            .create_surface(window.clone())
            .context("could not create a wgpu surface for the window")?;
        let adapter = self.pick_adapter(&surface)?;
        self.adapter_name = adapter.get_info().name.clone();
        let info = adapter.get_info();
        info!(
            "adapter: {} ({} / {}, {:?})",
            info.name, info.vendor, info.device, info.device_type
        );

        // `Limits::defaults()` is far too small: it caps storage buffers per shader stage at 8 and
        // `max_immediate_size` at 0, while `culling.wgsl` binds 12 storage buffers and seven
        // pipelines use `var<immediate>`. Ask for everything, then clamp to this adapter.
        let adapter_limits = adapter.limits();
        let limits = wgpu::Limits::unlimited().or_worse_values_from(&adapter_limits);
        let missing: Vec<&str> = crate::renderer::REQUIRED_FEATURES
            .difference(adapter.features())
            .iter_names()
            .map(|(name, _)| name)
            .collect();
        if !missing.is_empty() {
            warn!(
                "the adapter does not support: {} - the nanite pipeline will refuse to start",
                missing.join(", ")
            );
        }

        let device_desc = wgpu::DeviceDescriptor {
            label: Some("nanite-viewer-rs"),
            required_features: crate::renderer::REQUIRED_FEATURES,
            required_limits: limits,
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&device_desc))
            .map_err(|e| anyhow::anyhow!("could not create a logical device: {e}"))?;

        let config =
            pick_surface_config(&surface, &adapter, &device, self.args.width, self.args.height)
                .ok_or_else(|| anyhow::anyhow!("the surface has no usable configuration"))?;
        info!(
            "surface: {} x {} {:?}",
            config.width, config.height, config.format
        );

        let renderer = Renderer::new(
            device.clone(),
            queue.clone(),
            config.format,
            config.width,
            config.height,
            &self.env_ktx_path(),
        )
        .context("could not prepare the nanite renderer")?;

        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            None,
            None,
            None,
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, Default::default());

        self.window = Some(window);
        self.surface = Some(surface);
        self.adapter = Some(adapter);
        self.device = Some(device);
        self.queue = Some(queue);
        self.config = Some(config);
        self.renderer = Some(renderer);
        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);

        // `self.args` is borrowed by `load_model` through `self`, so the model name is cloned out
        // of the arguments first.
        let model = self.args.model.clone();
        self.load_model(&model)
    }

    /// Pick an adapter by hand instead of `request_adapter` so the feature requirement can be
    /// checked *before* a device is created, and so every candidate is logged: a machine with a
    /// virtual GPU (MirrorOp) or a software fallback otherwise silently picks something that
    /// cannot do 64-bit atomics.
    fn pick_adapter(&self, surface: &wgpu::Surface) -> Result<wgpu::Adapter> {
        let required = crate::renderer::REQUIRED_FEATURES;
        let adapters = pollster::block_on(self.instance.enumerate_adapters(wgpu::Backends::all()));
        let mut candidates: Vec<wgpu::Adapter> = adapters
            .into_iter()
            .filter(|adapter| adapter.is_surface_supported(surface))
            .collect();
        if candidates.is_empty() {
            let options = wgpu::RequestAdapterOptions {
                compatible_surface: Some(surface),
                ..Default::default()
            };
            // wgpu 30: `request_adapter` returns `Result<Adapter, RequestAdapterError>`, not
            // `Option` - the "no adapter" case is an error value now.
            if let Ok(adapter) = pollster::block_on(self.instance.request_adapter(&options)) {
                candidates.push(adapter);
            }
        }
        for adapter in &candidates {
            info!(
                "candidate: {} [{:?}] features ok: {}",
                adapter.get_info().name,
                adapter.get_info().device_type,
                adapter.features().contains(required)
            );
        }
        let score = |adapter: &wgpu::Adapter| -> i32 {
            let mut score = 0;
            if adapter.features().contains(required) {
                score += 100;
            }
            match adapter.get_info().device_type {
                wgpu::DeviceType::DiscreteGpu => score += 20,
                wgpu::DeviceType::IntegratedGpu => score += 10,
                wgpu::DeviceType::VirtualGpu => score -= 50,
                wgpu::DeviceType::Cpu => score -= 100,
                _ => {}
            }
            score
        };
        let best = candidates
            .iter()
            .max_by_key(|adapter| score(adapter))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no adapter here can present to this window - the nanite pipeline needs \
                     64-bit atomics, so a software or virtual GPU is not enough"
                )
            })?
            .clone();
        let missing = required.difference(best.features());
        if !missing.is_empty() {
            anyhow::bail!(
                "the nanite pipeline needs GPU features this device lacks: {}",
                missing
                    .iter_names()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Ok(best)
    }

    /// `<assets>/textures/hdr/gcanyon_cube.ktx`, the environment map `loadAssets()` uses.
    fn env_ktx_path(&self) -> PathBuf {
        self.model_dir
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("assets"))
            .join("textures")
            .join("hdr")
            .join("gcanyon_cube.ktx")
    }

    // ---------------------------------------------------------------- model loading

    fn source_mode(mode: SourceMode) -> load::SourceMode {
        match mode {
            SourceMode::Auto => load::SourceMode::Auto,
            SourceMode::Require => load::SourceMode::Require,
            SourceMode::Rebuild => load::SourceMode::Rebuild,
        }
    }

    /// `loadAssets()` for the nanite part: resolve the model, get an `Asset`, assemble the scene
    /// with `createScene2()`'s two instances, upload it, and frame it with `viewAll()`.
    fn load_model(&mut self, model: &str) -> Result<()> {
        let loaded = load::load_model(model, &self.model_dir, Self::source_mode(self.args.mode))
            .with_context(|| format!("could not load model {model}"))?;
        info!("[{}]", loaded.source.label());
        self.apply_model(loaded)
    }

    fn apply_model(&mut self, loaded: LoadedModel) -> Result<()> {
        let label = loaded.source.label();
        let scene = Scene::from_asset(loaded.asset.clone(), &load::default_instances())
            .context("the loaded model does not form a valid nanite scene")?;
        let summary = crate::scene::describe(&scene);
        let renderer = self
            .renderer
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("the renderer is not ready"))?;
        renderer.load_scene(&scene)?;
        info!("[scene] {label}: {summary}");
        self.model = Some(loaded);
        self.scene = Some(scene);

        // `viewAll()` at the end of `loadNaniteModel()` — start-up and reload frame identically.
        self.run_view_all();
        self.view_updated = true;
        Ok(())
    }

    /// `pbrtexture.cpp:1447` — the reload entry point. The Rust renderer keeps its pipelines,
    /// render targets and uniform buffers across a model change, so this is just a new scene:
    /// `Renderer::load_scene` replaces every nanite buffer the old model owned.
    fn reload_model(&mut self, model: &str) -> Result<()> {
        info!("[reload] {model}");
        if !Path::new(model).exists() && load::resolve_model_path(model, &self.model_dir).is_err() {
            anyhow::bail!("no such file: {model}");
        }
        self.load_model(model)
    }

    /// `handlePendingCommandLineReload()` (`pbrtexture.cpp:4300`): at most one reload per frame,
    /// in frame order, and a failure is logged rather than fatal.
    fn handle_pending_command_line_reload(&mut self) {
        let Some(reload) = load::take_due_reload(&mut self.args.reloads, self.frames_rendered) else {
            return;
        };
        if let Err(error) = self.reload_model(&reload.model) {
            error!("[reload] failed: {error:#}");
        }
    }

    // ---------------------------------------------------------------- frame loop

    /// `VulkanExampleBase::nextFrame()`.
    fn next_frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self.view_updated {
            self.view_updated = false;
            self.view_changed();
        }
        if self.view_all_requested {
            self.view_all_requested = false;
            self.run_view_all();
        }
        if self.reset_camera_requested {
            self.reset_camera_requested = false;
            let (width, height) = self.render_size();
            load::reset_camera(&mut self.camera, width, height);
            self.view_updated = true;
        }
        if self.open_dialog_requested {
            self.open_dialog_requested = false;
            match self.pick_model_dialog() {
                Ok(Some(path)) => {
                    let text = path.to_string_lossy().into_owned();
                    if let Err(error) = self.reload_model(&text) {
                        error!("[reload] failed: {error:#}");
                    }
                }
                Ok(None) => {}
                Err(error) => error!("[dialog] {error:#}"),
            }
        }
        self.handle_pending_command_line_reload();

        // The capture is a one-shot: `capture.pending` is set for exactly one frame and cleared
        // inside `submitFrame` (`vulkanexamplebase.cpp:783-790`), and the example exits right
        // after. `EventLoop::exit()` does *not* stop the current `about_to_wait` cycle, so without
        // this guard a second frame renders and overwrites the PNG that was just written - with an
        // empty one, because the occlusion state of the captured frame culls everything (see the
        // "frames after the first" note below).
        if self.capture_done {
            event_loop.exit();
            return Ok(());
        }

        let capturing = self.args.capture.is_some();
        let capture_last = capturing && self.frames_rendered + 1 >= self.args.capture_frames;

        self.draw(capture_last)?;

        if capturing {
            self.frames_rendered += 1;
        }
        self.frame_counter += 1;

        let now = Instant::now();
        let t_diff = now.duration_since(self.t_prev_end).as_secs_f64() * 1000.0;
        self.t_prev_end = now;
        self.frame_ms = t_diff;

        // `camera.update(frameTimer)` — after the frame was rendered, exactly like the base loop.
        let (forward, sideways) = self.camera_axes();
        if forward != 0.0 || sideways != 0.0 {
            self.camera
                .update(forward, sideways, (t_diff / 1000.0) as f32);
            self.view_updated = true;
        }

        self.fps_timer += t_diff;
        if self.fps_timer > 1000.0 {
            self.last_fps = (self.frame_counter as f64 * 1000.0 / self.fps_timer) as f32;
            if !self.overlay_active() {
                let title = self.window_title();
                if let Some(window) = &self.window {
                    window.set_title(&title);
                }
            }
            self.frame_counter = 0;
            self.fps_timer = 0.0;
        }

        if self.capture_done {
            event_loop.exit();
        }
        Ok(())
    }

    /// `viewChanged()` in pbrtexture only forces the uniform buffers to be rewritten, which
    /// `Renderer::render` does at the top of every frame anyway — so there is nothing to do here.
    fn view_changed(&mut self) {}

    /// W/S forward, A/D sideways, matching the base framework's first-person key handling.
    fn camera_axes(&self) -> (f32, f32) {
        let forward = f32::from(self.keys.up) - f32::from(self.keys.down);
        let sideways = f32::from(self.keys.right) - f32::from(self.keys.left);
        (forward, sideways)
    }

    /// The size everything renders at. The capture renders at the same size as the swap chain,
    /// which is what `--capture` compares against a C++ capture.
    fn render_size(&self) -> (u32, u32) {
        (self.args.width, self.args.height)
    }

    /// `settings.overlay` in the base framework: `--capture` turns the overlay off
    /// (`vulkanexamplebase.cpp:221`), which is why the visualization tests need `--visualize`
    /// instead of the slider.
    fn overlay_active(&self) -> bool {
        self.args.capture.is_none()
    }

    fn draw(&mut self, capture_last: bool) -> Result<()> {
        let window = self.window.clone().ok_or_else(|| anyhow::anyhow!("no window"))?;
        let device = self.device.clone().ok_or_else(|| anyhow::anyhow!("no device"))?;
        let queue = self.queue.clone().ok_or_else(|| anyhow::anyhow!("no queue"))?;

        let (width, height) = self.render_size();
        // Every `self.renderer` borrow below is scoped: `render_egui` needs `&mut self`, and the
        // render targets have to be finished (and submitted) before the overlay pass is recorded.
        {
            let renderer = self
                .renderer
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("no renderer"))?;
            if renderer.width() != width || renderer.height() != height {
                renderer.resize(width, height)?;
            }
        }

        let capturing = self.args.capture.is_some();
        // `printRasterStatsHeadless()` early-returns unless `capture.active`
        // (`pbrtexture.cpp:4016`), so `--raster-stats` only ever prints during a capture.
        let read_stats = self.args.raster_stats && capturing;

        if capturing {
            let stale = match &self.capture_target {
                Some(target) => target.width != width || target.height != height,
                None => true,
            };
            if stale {
                // The colour format has to be the one every render pipeline was created with, i.e.
                // the surface format - see `capture::CaptureTarget::new`.
                let format = self
                    .config
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no surface configuration"))?
                    .format;
                self.capture_target = Some(CaptureTarget::new(&device, width, height, format));
            }
            let (color_view, depth_view) = {
                let target = self
                    .capture_target
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no capture target"))?;
                (target.color_view(), target.depth_view())
            };
            let stats = {
                let renderer = self
                    .renderer
                    .as_mut()
                    .ok_or_else(|| anyhow::anyhow!("no renderer"))?;
                renderer.render(&self.camera, &self.settings, &color_view, &depth_view, read_stats)?
            };
            self.last_stats = stats;

            if capture_last {
                // `submitFrame()` saves the last captured image *before* presenting it
                // (`vulkanexamplebase.cpp`), and there is nothing to present here at all.
                let path = self
                    .args
                    .capture
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("capture path vanished"))?;
                let target = self
                    .capture_target
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no capture target"))?;
                let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("capture.readback"),
                });
                let image = target.read_back(&device, &queue, encoder)?;
                crate::capture::write_png(&path, &image)?;
                info!("[capture] wrote {} x {} to {}", width, height, path.display());
                self.capture_done = true;
            }
            return Ok(());
        }

        // `get_current_texture` borrows the surface, but the returned texture does not, so the
        // borrow is over before a possible `configure_surface(&mut self)`.
        let current = {
            let surface = self
                .surface
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no surface"))?;
            surface.get_current_texture()
        };
        let surface_texture = match current {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(())
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                // `prepareFrame()` treats OUT_OF_DATE like a resize and skips the frame.
                self.configure_surface(width, height)?;
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                anyhow::bail!("the surface could not be acquired: validation error")
            }
        };

        let color_view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let stats = {
            let renderer = self
                .renderer
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("no renderer"))?;
            // `frame_depth_view` must be taken *after* any resize and must not be cached - it is the
            // renderer's own depth attachment, recreated whenever the extent changes.
            let depth_view = renderer.frame_depth_view();
            renderer.render(&self.camera, &self.settings, &color_view, &depth_view, read_stats)?
        };
        self.last_stats = stats;

        self.render_egui(&device, &queue, &window, &surface_texture.texture)?;

        queue.present(surface_texture);
        Ok(())
    }

    /// `Surface::configure` panics while a `SurfaceTexture` is alive, so every caller of this has
    /// already dropped its texture.
    fn configure_surface(&mut self, width: u32, height: u32) -> Result<()> {
        let device = self.device.clone().ok_or_else(|| anyhow::anyhow!("no device"))?;
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no surface"))?;
        let adapter = self
            .adapter
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no adapter"))?;
        let config = pick_surface_config(surface, adapter, &device, width, height)
            .ok_or_else(|| anyhow::anyhow!("the surface has no usable configuration"))?;
        surface.configure(&device, &config);
        // `SurfaceConfiguration` is not `Copy`, so read it before handing it to `self`.
        self.args.width = config.width;
        self.args.height = config.height;
        self.config = Some(config);
        Ok(())
    }

    /// The egui overlay: `updateOverlay()` + `OnUpdateUIOverlay()`.
    ///
    /// It runs in its own render pass on the same colour view with **no** depth attachment —
    /// `egui_wgpu::RendererOptions::depth_stencil_format` is `None`, so its pipeline has no depth
    /// state at all and would fail validation if handed one.
    ///
    /// The two egui objects are `take()`n out of `self` for the duration: `Context::run_ui` calls
    /// the closure with `&mut Ui`, that closure wants `&mut self`, and `egui_state` /
    /// `egui_renderer` are fields of `self` — so the fields have to be parked in locals first.
    fn render_egui(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        window: &Arc<Window>,
        color_texture: &wgpu::Texture,
    ) -> Result<()> {
        let mut state = self
            .egui_state
            .take()
            .ok_or_else(|| anyhow::anyhow!("no egui state"))?;
        let mut egui_renderer = self
            .egui_renderer
            .take()
            .ok_or_else(|| anyhow::anyhow!("no egui renderer"))?;

        let result = self.render_egui_inner(&mut state, &mut egui_renderer, device, queue, window, color_texture);

        self.egui_state = Some(state);
        self.egui_renderer = Some(egui_renderer);
        result
    }

    fn render_egui_inner(
        &mut self,
        state: &mut egui_winit::State,
        egui_renderer: &mut egui_wgpu::Renderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        window: &Arc<Window>,
        color_texture: &wgpu::Texture,
    ) -> Result<()> {
        let raw_input = state.take_egui_input(window);
        // `Context::run_ui` is egui 0.36's replacement for the removed `Context::run`: it returns a
        // `FullOutput` whose texture deltas the caller must apply by hand (0.36.2 has no
        // `on_full_output`).
        let ctx = self.egui_ctx.clone();
        let full_output = ctx.run_ui(raw_input, |ui| {
            // `updateOverlay()` puts everything in an ImGui window called "Vulkan Example" at
            // (10*scale, 10*scale), auto-resized, not movable and not resizable. egui points already
            // include the DPI scale, so the offset is a plain 10.
            egui::Window::new("Vulkan Example")
                .default_pos([10.0, 10.0])
                .resizable(false)
                .movable(false)
                .show(ui.ctx(), |ui| self.update_ui_overlay(ui));
        });

        state.handle_platform_output(window, full_output.platform_output);
        // `TexturesDelta::set` maps a texture id to a *list* of image deltas.
        for (id, deltas) in &full_output.textures_delta.set {
            for delta in deltas {
                egui_renderer.update_texture(device, queue, *id, delta);
            }
        }

        let primitives = ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.args.width, self.args.height],
            pixels_per_point: full_output.pixels_per_point,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui"),
        });
        // `update_buffers` must come before `render`, which panics otherwise. It returns command
        // buffers of its own (empty unless a paint callback is registered) that have to be submitted
        // alongside the pass.
        let extra = egui_renderer.update_buffers(device, queue, &mut encoder, &primitives, &screen);
        {
            let view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
            // wgpu 30 has no `Device::create_render_pass` — the pass comes from the encoder, which
            // is why it is dropped before `encoder.finish()`.
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    // New in wgpu 30.
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // `forget_lifetime` is required: egui-wgpu's `render` wants a `RenderPass<'static>`.
            egui_renderer.render(&mut pass.forget_lifetime(), &primitives, &screen);
        }
        for id in &full_output.textures_delta.free {
            egui_renderer.free_texture(id);
        }
        queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        Ok(())
    }

    /// `VulkanExample::OnUpdateUIOverlay()` (`pbrtexture.cpp:4404`), control for control.
    fn update_ui_overlay(&mut self, ui: &mut egui::Ui) {
        ui.label(WINDOW_TITLE);
        ui.label(self.adapter_name.clone());
        ui.label(format!(
            "{:.2} ms/frame ({:.0} fps)",
            if self.last_fps > 0.0 {
                1000.0 / f64::from(self.last_fps)
            } else {
                self.frame_ms
            },
            self.last_fps
        ));

        let stats = self.last_stats;
        ui.label(format!(
            "Num triangles without vulkanite: {}",
            stats.scene_triangles()
        ));
        ui.label(format!("Num triangles hw raserized: {}", stats.hw_triangles()));
        ui.label(format!("Num triangles sw raserized: {}", stats.sw_triangles()));
        ui.label(format!(
            "Num triangles raserized in total: {}",
            stats.total_triangles()
        ));

        // `ImGui::TreeNodeEx("Settings", ImGuiTreeNodeFlags_DefaultOpen)` — open by default. Note
        // that `Ui::collapsing` is *not*: it builds a plain `CollapsingHeader`, which starts closed.
        egui::CollapsingHeader::new("Settings")
            .default_open(true)
            .show(ui, |ui| {
                // `ImGui::PushItemWidth(110 * scale)`. In egui 0.36 `slider_width` lives on the
                // spacing struct, not on `Style` itself.
                ui.style_mut().spacing.slider_width = 110.0;

                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings.exposure)
                            .speed(0.1)
                            .range(0.1..=2.0),
                    );
                    ui.label("Exposure");
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings.gamma)
                            .speed(0.1)
                            .range(0.1..=2.0),
                    );
                    ui.label("Gamma");
                });

                ui.checkbox(&mut self.settings.skybox, "Skybox");
                // `vis_topView` swaps to a 1/5-size render pass (`pbrtexture.cpp:1022`); the Rust
                // renderer has no second render pass, so the control is accepted and inert.
                let mut top_view = false;
                ui.checkbox(&mut top_view, "Top View")
                    .on_hover_text("not implemented in the Rust viewer (pbrtexture.cpp:1022)");
                ui.checkbox(&mut self.settings.software_raster, "Software Rasterization");
                ui.checkbox(&mut self.settings.frustum_occlusion, "Frustrum&Occlusion Culling");

                ui.add(egui::Slider::new(&mut self.threshold_int, 0..=1000).text("Threshold"));
                self.settings.threshold = self.threshold_int as f32 * THRESHOLD_INT_DIV;

                ui.add(
                    egui::Slider::new(&mut self.settings.visualize, 0..=3)
                        .text("Visualize Clusters"),
                );

                let lod_max = self
                    .model
                    .as_ref()
                    .map(|model| model.asset.lod_nums.max(1) as i32 - 1)
                    .unwrap_or(0);
                // Inert in the C++ viewer too: `vis_clusters_level` is only read from commented-out
                // code (`pbrtexture.cpp:923`), so it is displayed and disabled here as well.
                ui.add_enabled_ui(false, |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.lod_level, 0..=lod_max.max(0))
                            .text("LOD level"),
                    );
                });

                if ui.button("Load model...").clicked() {
                    self.open_dialog_requested = true;
                }
                ui.label("or press O");
                if ui.button("View all").clicked() {
                    self.view_all_requested = true;
                }
                ui.label("or press V - frames the loaded model; applied after start and reload too");
                if ui.button("Reset camera").clicked() {
                    self.reset_camera_requested = true;
                }
            });
    }

    // ---------------------------------------------------------------- camera

    fn run_view_all(&mut self) {
        let (width, height) = self.render_size();
        let Some(scene) = self.scene.as_ref() else {
            warn!("[view all] no model loaded");
            return;
        };
        if load::view_all(scene, &mut self.camera, width, height).is_none() {
            warn!("[view all] the scene has no bounds - keeping the current camera");
        }
        self.view_updated = true;
    }

    // ---------------------------------------------------------------- input

    /// `VulkanExampleBase::handleMouseMove()` (`vulkanexamplebase.cpp:3300`).
    ///
    /// The sign is the trap: the C++ code computes `dx = previous - current`, *not*
    /// `current - previous`, and then feeds `dy` to pitch and `-dx` to yaw.
    fn mouse_moved(&mut self, x: f64, y: f64) {
        let egui_grabbed = self.egui_ctx.egui_wants_pointer_input();
        let (px, py) = match self.mouse_pos.replace((x, y)) {
            Some(previous) => previous,
            None => return,
        };
        if egui_grabbed {
            return;
        }
        let dx = (px - x) as f32;
        let dy = (py - y) as f32;
        let buttons = self.mouse_buttons;
        if buttons.left {
            self.camera
                .rotate(Vec3::new(dy * ROTATION_SPEED, -dx * ROTATION_SPEED, 0.0));
            self.view_updated = true;
        }
        if buttons.right {
            self.camera
                .translate(Vec3::new(0.0, 0.0, dy * TRANSLATE_SPEED));
            self.view_updated = true;
        }
        if buttons.middle {
            self.camera
                .translate(Vec3::new(-dx * TRANSLATE_SPEED, -dy * TRANSLATE_SPEED, 0.0));
            self.view_updated = true;
        }
    }

    /// `handlePendingModelDialog()` (`pbrtexture.cpp:4326`): the same filters, the same title,
    /// starting in `assets/models`.
    fn pick_model_dialog(&mut self) -> Result<Option<PathBuf>> {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Load model")
            .add_filter("models (glTF, nanite blob)", &["gltf", "glb", "fcc"])
            .add_filter("glTF models", &["gltf", "glb"])
            .add_filter("nanite blobs", &["fcc"])
            .add_filter("All files", &["*"]);
        if self.model_dir.is_dir() {
            dialog = dialog.set_directory(&self.model_dir);
        }
        Ok(dialog.pick_file())
    }

    /// `VulkanExampleBase::getWindowTitle()`: title, device name, and the frame counter while the
    /// overlay is off.
    fn window_title(&self) -> String {
        let mut title = String::from(WINDOW_TITLE);
        if !self.adapter_name.is_empty() {
            title.push_str(" - ");
            title.push_str(&self.adapter_name);
        }
        if !self.overlay_active() {
            title.push_str(&format!(" - {} fps", self.last_fps.max(0.0) as u32));
        }
        title
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.resume(event_loop) {
            error!("{error:#}");
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Ask egui *before* handing it the event: `egui_wants_pointer_input` reflects the previous
        // frame, which is what the C++ code's `io.WantCaptureMouse` is too.
        let egui_grabbed = self.egui_ctx.egui_wants_pointer_input();
        if let (Some(state), Some(window)) = (self.egui_state.as_mut(), self.window.as_ref()) {
            // `EventResponse` says whether egui wants a repaint. The loop runs with
            // `ControlFlow::Poll` and `about_to_wait` always requests a redraw, so there is nothing
            // to act on — but the return value is `#[must_use]`.
            let _response = state.on_window_event(&**window, &event);
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let width = size.width.max(1);
                let height = size.height.max(1);
                if width != self.args.width || height != self.args.height {
                    if let Err(error) = self.configure_surface(width, height) {
                        error!("[resize] {error:#}");
                        return;
                    }
                    let aspect = width as f32 / height as f32;
                    self.camera.update_aspect_ratio(aspect);
                    self.view_updated = true;
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => self.mouse_buttons.left = pressed,
                    MouseButton::Right => self.mouse_buttons.right = pressed,
                    MouseButton::Middle => self.mouse_buttons.middle = pressed,
                    _ => {}
                }
                // A drag must not jump: forget the previous position when a button goes down, and
                // never let egui's drag start a camera move.
                if pressed && !egui_grabbed {
                    self.mouse_pos = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.mouse_moved(position.x, position.y),
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if let Key::Character(c) = &event.logical_key {
                    match c.as_str() {
                        "w" | "W" => self.keys.up = pressed,
                        "s" | "S" => self.keys.down = pressed,
                        "a" | "A" => self.keys.left = pressed,
                        "d" | "D" => self.keys.right = pressed,
                        "o" | "O" if pressed => self.open_dialog_requested = true,
                        "v" | "V" if pressed => self.view_all_requested = true,
                        _ => {}
                    }
                }
                // winit has no `Key::Escape`; the named keys are wrapped in `Key::Named`.
                if event.logical_key == Key::Named(NamedKey::Escape) && pressed {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.next_frame(event_loop) {
            error!("{error:#}");
            event_loop.exit();
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Run the viewer. `main.rs` is a thin wrapper around this.
pub fn run(args: Args) -> Result<()> {
    let event_loop = EventLoop::new().context("winit could not create an event loop")?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(args);
    event_loop
        .run_app(&mut app)
        .map_err(|e| anyhow::anyhow!("the event loop stopped: {e}"))?;
    Ok(())
}

/// The blob extension, re-exported so `main.rs` can describe `--model bunny.fcc`.
pub const BLOB_EXTENSION: &str = nanite_blob::NANITE_BLOB_EXTENSION;

/// Chooses and applies a surface configuration, mirroring the C++ swap chain.
///
/// `base/VulkanSwapChain.cpp:178-186` walks a *preference list* —
/// `VK_FORMAT_B8G8R8A8_UNORM`, `VK_FORMAT_R8G8B8A8_UNORM`,
/// `VK_FORMAT_A8B8G8R8_UNORM_PACK32` — and takes the first one the surface offers, falling back to
/// `surfaceFormats[0]`. Every entry is **UNORM, never sRGB**, and the whole renderer (pipelines,
/// the visibility buffer merge, the tonemap/gamma in `shading.frag`, and
/// `saveSwapchainImage`, which writes the capture PNG straight from those bytes) is built against
/// that. wgpu's `Surface::get_default_config` instead takes `caps.formats.first()`, which on this
/// machine is `Bgra8UnormSrgb` — a different *transfer function*, so the pipelines and an offscreen
/// capture target would not even be compatible, and egui complains about it as well
/// ("egui prefers Rgba8Unorm or Bgra8Unorm").
///
/// wgpu has no `Abgr8Unorm`, so the third C++ preference cannot be expressed; it does not occur on
/// any Vulkan swap chain anyway.
///
/// The present mode follows `settings.vsync = false` (`base/vulkanexamplebase.h:198`): the C++ then
/// prefers `IMMEDIATE`/`MAILBOX` over the mandatory `FIFO`, which is wgpu's `AutoNoVsync`.
///
/// `Surface::configure` panics when the format is not in `caps.formats`, so the lookup and the
/// configure call belong together.
fn pick_surface_config(
    surface: &wgpu::Surface<'_>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Option<wgpu::SurfaceConfiguration> {
    let caps = surface.get_capabilities(adapter);
    let preferred = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm];
    let format = preferred
        .iter()
        .find(|format| caps.formats.contains(format))
        .copied()
        .or_else(|| caps.formats.first().copied())?;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: width.max(1),
        height: height.max(1),
        desired_maximum_frame_latency: 2,
        present_mode: wgpu::PresentMode::AutoNoVsync,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    };
    surface.configure(device, &config);
    Some(config)
}

/// `defaultModelMats()`, reachable from tests — the two instance transforms have to match
/// `createScene2()` exactly or the captures cannot line up with the C++ goldens.
pub fn default_instances() -> Vec<Mat4> {
    load::default_instances()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &str) -> Args {
        Args::parse(words.split_whitespace().map(String::from)).unwrap()
    }

    #[test]
    fn parse_matches_the_c_flag_set() {
        let args = parse(
            "--model bunny --nanite-blob --visualize 2 --threshold 0 --raster-stats \
             --capture out.png --captureframes 11",
        );
        assert_eq!(args.model, "bunny");
        assert_eq!(args.mode, SourceMode::Require);
        assert_eq!(args.visualize, 2);
        assert_eq!(args.threshold, 0, "--threshold 0 must stay 0");
        assert!(args.raster_stats);
        assert_eq!(args.capture.as_deref(), Some(Path::new("out.png")));
        assert_eq!(args.capture_frames, 11);
    }

    #[test]
    fn bare_model_name_is_positional() {
        let args = Args::parse(vec![String::from("bunny.fcc")]).unwrap();
        assert_eq!(args.model, "bunny.fcc");
        assert_eq!(args.mode, SourceMode::Auto);
        assert_eq!(args.width, DEFAULT_WIDTH);
        assert_eq!(args.height, DEFAULT_HEIGHT);
    }

    #[test]
    fn reloads_are_paired_and_sorted_by_frame() {
        let args = parse("--reload a.gltf --reloadframe 8 --reload2 b.fcc --reload2frame 5");
        assert_eq!(args.reloads.len(), 2);
        assert_eq!(args.reloads[0].model, "b.fcc");
        assert_eq!(args.reloads[0].frame, 5);
        assert_eq!(args.reloads[1].model, "a.gltf");
        assert_eq!(args.reloads[1].frame, 8);
    }

    #[test]
    fn reload_frame_defaults_match_the_viewer() {
        let args = parse("--reload a.gltf");
        assert_eq!(args.reloads[0].frame, 10);
        // `--reloadframe 0` becomes 1 (`max(1, getValueAsInt(...))`).
        let args = parse("--reload a.gltf --reloadframe 0");
        assert_eq!(args.reloads[0].frame, 1);
    }

    #[test]
    fn visualize_and_threshold_are_clamped_like_the_c_viewer() {
        assert_eq!(parse("--visualize 9").visualize, 3);
        assert_eq!(parse("--visualize -3").visualize, 0);
        assert_eq!(parse("--threshold 5000").threshold, 1000);
        assert_eq!(parse("--threshold 0").threshold, 0);
    }

    #[test]
    fn unknown_option_is_an_error() {
        assert!(Args::parse(vec![String::from("--nope")]).is_err());
        assert!(Args::parse(vec![String::from("--reloadframe"), String::from("5")]).is_err());
    }

    #[test]
    fn default_threshold_matches_the_c_default() {
        let args = Args::default();
        assert_eq!(args.threshold, 500);
        assert!((args.threshold as f32 * THRESHOLD_INT_DIV - 0.0005).abs() < 1e-9);
    }

    #[test]
    fn parse_int_keeps_zero_and_negatives() {
        assert_eq!(parse_int("0", 500), 0);
        assert_eq!(parse_int("-4", 500), -4);
        assert_eq!(parse_int("abc", 500), 500);
    }

    #[test]
    fn source_mode_maps_every_variant() {
        assert_eq!(App::source_mode(SourceMode::Auto), load::SourceMode::Auto);
        assert_eq!(App::source_mode(SourceMode::Require), load::SourceMode::Require);
        assert_eq!(App::source_mode(SourceMode::Rebuild), load::SourceMode::Rebuild);
    }

    #[test]
    fn default_instances_are_the_two_of_createScene2() {
        let mats = default_instances();
        assert_eq!(mats.len(), 2);
        assert_eq!(mats[1], Mat4::IDENTITY);
        // Instance 0 is `T(0,0,3) * Ry(-90)`: the translation column is (0, 0, 3).
        assert_eq!(mats[0].w_axis.truncate(), Vec3::new(0.0, 0.0, 3.0));
        // ... and its rotation is a -90 degree turn about Y, so local +Z maps to world -X
        // (`Ry(t)` sends +Z to (sin t, 0, cos t)).
        let rotated = mats[0].transform_vector3(Vec3::Z);
        assert!((rotated + Vec3::X).length() < 1e-5);
    }
}
