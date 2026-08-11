//! The native window (ADR-037, docs/33, TD-60).
//!
//! docs/33 is explicit that this is a **native windowed app — winit + wgpu for
//! the document surface — and *not* a webview**, because "Excel-grade latency,
//! memory, and IME behavior are the reasons users stay on desktop". This module
//! is the winit half of that sentence.
//!
//! # What is deliberately *not* here
//! No decisions. This translates winit events into [`crate::input::Intent`]s,
//! hands them to [`App`], and presents whatever quads come back. Every rule
//! about what a key means, where the cursor goes and what an edit writes lives
//! in `input.rs` and `app.rs`, where it can be tested without a display. If a
//! behaviour can only be exercised by opening a window, it is in the wrong
//! file.
//!
//! # Redraw policy
//! `ControlFlow::Wait` and a redraw only when something changed. A spreadsheet
//! at rest is a static image; a 120 Hz loop redrawing it would burn the battery
//! docs/31 budgets ("<8% per hour of active editing") to produce identical
//! frames. The frame *budget* is docs/31's 8.3 ms; the frame *rate* is however
//! often the user does something.

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::app::App;
use crate::gpu::{Present, Renderer};
use crate::input::{self, Key, Mods};

/// Lines a wheel notch scrolls, in rows. Three is the platform convention on
/// both Windows and macOS and is what a user's hand expects.
const WHEEL_ROWS: f32 = 3.0;

pub struct Shell {
    app: App,
    window: Option<Arc<Window>>,
    gpu: Option<(Renderer, Present)>,
    mods: Mods,
    dragging: bool,
    /// Last cursor position in logical pixels.
    pointer: (f32, f32),
    /// Per-frame cost in milliseconds — the number TD-60 exists to obtain,
    /// since an offscreen render measures a readback a presenting frame never
    /// does. Scene build plus encode and submit; the vsync wait is in
    /// `acquire` and is deliberately not counted as cost.
    timings: Vec<f64>,
    cpu: Vec<f64>,
    acquire: Vec<f64>,
    /// When set, the shell presents this many frames and exits, printing the
    /// timings. This is how a windowed frame gets measured on a machine nobody
    /// is sitting at, and it presents to the **real compositor** — it is not a
    /// second, quieter offscreen path.
    frame_budget: Option<usize>,
    /// Scrolls one row per frame in budget mode, so the measured frames are
    /// scroll frames and not a repainted still.
    scrolling: bool,
    failure: Option<String>,
}

impl Shell {
    pub fn new(app: App) -> Shell {
        Shell {
            app,
            window: None,
            gpu: None,
            mods: Mods::NONE,
            dragging: false,
            pointer: (0.0, 0.0),
            timings: Vec::new(),
            cpu: Vec::new(),
            acquire: Vec::new(),
            frame_budget: None,
            scrolling: false,
            failure: None,
        }
    }

    /// Presents `frames` frames, scrolling one row between each, then exits.
    pub fn measuring(mut self, frames: usize) -> Shell {
        self.frame_budget = Some(frames);
        self.scrolling = true;
        self
    }

    /// Runs the event loop. Returns a report when a frame budget was set.
    pub fn run(mut self) -> Result<String, String> {
        let event_loop = EventLoop::new().map_err(|e| format!("creating the event loop: {e}"))?;
        event_loop.set_control_flow(if self.frame_budget.is_some() {
            ControlFlow::Poll
        } else {
            ControlFlow::Wait
        });
        let measuring = self.frame_budget.is_some();
        event_loop
            .run_app(&mut self)
            .map_err(|e| format!("running the event loop: {e}"))?;
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        if !measuring {
            return Ok(String::from("window closed"));
        }
        Ok(self.report())
    }

    fn report(&mut self) -> String {
        let pct = |v: &mut Vec<f64>, p: f64| -> f64 {
            if v.is_empty() {
                return f64::NAN;
            }
            v.sort_by(f64::total_cmp);
            v[((v.len() as f64 - 1.0) * p) as usize]
        };
        let (pw, ph) = self
            .gpu
            .as_ref()
            .map(|(_, p)| p.physical_size())
            .unwrap_or((0, 0));
        let scale = self.gpu.as_ref().map(|(_, p)| p.scale).unwrap_or(1.0);
        let n = self.timings.len();
        let cpu50 = pct(&mut self.cpu, 0.50);
        let cpu99 = pct(&mut self.cpu, 0.99);
        let t50 = pct(&mut self.timings, 0.50);
        let t99 = pct(&mut self.timings, 0.99);
        let a50 = pct(&mut self.acquire, 0.50);
        let a99 = pct(&mut self.acquire, 0.99);
        let jank = self.timings.iter().filter(|t| **t > 8.3).count();
        let (worst_at, worst) =
            self.timings
                .iter()
                .enumerate()
                .fold(
                    (0usize, 0.0f64),
                    |acc, (i, t)| {
                        if *t > acc.1 {
                            (i, *t)
                        } else {
                            acc
                        }
                    },
                );
        let first = self.timings.first().copied().unwrap_or(f64::NAN);
        format!(
            "W-PRESENT (TD-60) - {pw}x{ph} physical at {scale}x, {n} presented scroll frames
           scene build      p50 {cpu50:.3} ms   p99 {cpu99:.3} ms   (viewport + scene, CPU)
           frame cost       p50 {t50:.3} ms   p99 {t99:.3} ms   (scene + encode + submit)
           budget           8.3 ms (docs/31, scroll frame) - {jank} of {n} frames over it
           worst frame      {worst:.3} ms at index {worst_at} (frame 0: {first:.3} ms,
                            the one that uploads the 1 MiB glyph atlas)
           vsync wait       p50 {a50:.3} ms   p99 {a99:.3} ms   (blocked in
                            get_current_texture under Fifo - the display's pace,
                            not the renderer's, and deliberately not counted as cost)
           present mode     Fifo (vsync)"
        )
    }

    fn redraw(&mut self) {
        let Some((renderer, present)) = self.gpu.as_mut() else {
            return;
        };
        let started = Instant::now();
        let quads = self.app.frame();
        let cpu = started.elapsed().as_secs_f64() * 1000.0;
        if let Some((size, bytes)) = self.app.take_atlas_upload() {
            renderer.upload_atlas(size, bytes);
        }
        match renderer.present(present, &quads) {
            Ok(timing) => {
                self.cpu.push(cpu);
                // The frame's *cost*: everything the application does, and
                // nothing the display makes it wait for.
                self.timings.push(cpu + timing.submit_ms);
                self.acquire.push(timing.acquire_ms);
            }
            // `Timeout` is a compositor hiccup and the next frame fixes it;
            // anything else is worth naming rather than swallowing.
            Err(err) => {
                if !matches!(err, wgpu::SurfaceError::Timeout) {
                    self.failure = Some(format!("presenting a frame: {err}"));
                }
            }
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Ehkatra")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                self.failure = Some(format!("creating the window: {err}"));
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        match Renderer::for_surface(window.clone(), size.width, size.height, scale) {
            Ok((renderer, present)) => {
                if !renderer.format_is_srgb() {
                    eprintln!(
                        "ehkatra-shell: this display offers no sRGB surface format ({:?}); \
                         colours will be lighter than the theme specifies",
                        renderer.format()
                    );
                }
                let (lw, lh) = present.logical_size();
                self.app.resize(lw, lh, present.scale);
                self.gpu = Some((renderer, present));
            }
            Err(err) => {
                self.failure = Some(err);
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some((renderer, present)) = self.gpu.as_mut() {
                    let scale = present.scale;
                    renderer.reconfigure(present, size.width, size.height, scale);
                    let (lw, lh) = present.logical_size();
                    self.app.resize(lw, lh, scale);
                }
                self.request_redraw();
            }
            // Per-monitor DPI (docs/33 §Displays). The scene is authored in
            // logical pixels, so the only thing that changes is the scale the
            // glyphs rasterise at — the atlas keys on it, so both densities
            // coexist and a window dragged between monitors does not flash.
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some((renderer, present)) = self.gpu.as_mut() {
                    let (w, h) = present.physical_size();
                    renderer.reconfigure(present, w, h, scale_factor as f32);
                    let (lw, lh) = present.logical_size();
                    self.app.resize(lw, lh, scale_factor as f32);
                }
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(state) => {
                let state: ModifiersState = state.state();
                self.mods = Mods {
                    ctrl: state.control_key(),
                    shift: state.shift_key(),
                    alt: state.alt_key(),
                };
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let Some(key) = translate_key(&event.logical_key) else {
                    return;
                };
                let Some(intent) = input::translate(key, self.mods, self.app.mode()) else {
                    return;
                };
                if self.app.handle(intent).redraw {
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        (-x * WHEEL_ROWS * 64.0, -y * WHEEL_ROWS * 20.0)
                    }
                    MouseScrollDelta::PixelDelta(p) => {
                        let scale = self.gpu.as_ref().map(|(_, s)| s.scale).unwrap_or(1.0) as f64;
                        (-(p.x / scale) as f32, -(p.y / scale) as f32)
                    }
                };
                if self.app.scroll(dx, dy).redraw {
                    self.request_redraw();
                }
            }
            // winit reports the button without a position and the position
            // without a button, so the last cursor position is remembered and
            // the press is resolved against it.
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                self.dragging = state == ElementState::Pressed;
                let (x, y) = self.pointer;
                let outcome = if self.dragging {
                    self.app.pointer_down(x, y, self.mods.shift)
                } else {
                    // The release is where a fill drag commits, so it is an
                    // event in its own right and not merely the end of one.
                    self.app.pointer_up()
                };
                if outcome.redraw {
                    self.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.gpu.as_ref().map(|(_, s)| s.scale).unwrap_or(1.0) as f64;
                self.pointer = ((position.x / scale) as f32, (position.y / scale) as f32);
                if !self.dragging {
                    return;
                }
                let (x, y) = self.pointer;
                if self.app.pointer_drag(x, y).redraw {
                    self.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                if let Some(budget) = self.frame_budget {
                    if self.timings.len() >= budget || self.failure.is_some() {
                        event_loop.exit();
                        return;
                    }
                    if self.scrolling {
                        self.app.scroll(0.0, 20.0);
                    }
                    self.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn translate_key(key: &WinitKey) -> Option<Key> {
    Some(match key {
        WinitKey::Named(NamedKey::ArrowLeft) => Key::Left,
        WinitKey::Named(NamedKey::ArrowRight) => Key::Right,
        WinitKey::Named(NamedKey::ArrowUp) => Key::Up,
        WinitKey::Named(NamedKey::ArrowDown) => Key::Down,
        WinitKey::Named(NamedKey::Home) => Key::Home,
        WinitKey::Named(NamedKey::End) => Key::End,
        WinitKey::Named(NamedKey::PageUp) => Key::PageUp,
        WinitKey::Named(NamedKey::PageDown) => Key::PageDown,
        WinitKey::Named(NamedKey::Enter) => Key::Enter,
        WinitKey::Named(NamedKey::Tab) => Key::Tab,
        WinitKey::Named(NamedKey::Escape) => Key::Escape,
        WinitKey::Named(NamedKey::F2) => Key::F2,
        WinitKey::Named(NamedKey::Delete) => Key::Delete,
        WinitKey::Named(NamedKey::Backspace) => Key::Backspace,
        WinitKey::Named(NamedKey::Space) => Key::Character(' '),
        // The *text* the layout produced, not a scancode — so a French or
        // Cyrillic keyboard needs nothing here, and neither will a native IME
        // when it lands (docs/33: composition through the in-cell overlay).
        WinitKey::Character(text) => Key::Character(text.chars().next()?),
        _ => return None,
    })
}
