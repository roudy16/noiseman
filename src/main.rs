#![windows_subsystem = "windows"]

mod audio;
mod noise;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(not(target_os = "linux"))]
mod tray {
    pub enum Tray {}

    pub fn spawn(_: winit::event_loop::EventLoopProxy<crate::Event>, _: crate::TrayState) {}

    impl Tray {
        pub fn sync(&mut self, _: crate::TrayState) {
            match *self {}
        }
    }
}

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use font8x8::legacy::BASIC_LEGACY;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use noise::Kind;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Event {
    AudioLost,
    ToggleWindow,
    ToggleMute,
    Quit,
    TrayGone,
    TrayReady(tray::Tray),
}

// Fields are only read by tray.rs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, PartialEq)]
struct TrayState {
    shown: bool,
    muted: bool,
    kind: Kind,
}

fn send_or_log(proxy: &EventLoopProxy<Event>, ev: Event, what: &str) {
    if let Err(e) = proxy.send_event(ev) {
        eprintln!("noiseman: cannot {what}, event loop gone: {e}");
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

const KINDS: usize = Kind::ALL.len();
const PAD: i32 = 8;
const GAP: i32 = 4;
const BTN_W: i32 = 56;
const ROW_H: i32 = 20;
const WIN_W: i32 = PAD * 2 + KINDS as i32 * (BTN_W + GAP) - GAP;
const WIN_H: i32 = 124;
const BAR: Rect = Rect { x: PAD, y: 40, w: 240, h: ROW_H };
const MUTE: Rect = Rect { x: PAD, y: 72, w: 64, h: ROW_H };
const STATUS_Y: i32 = 104;
const BG: u32 = 0x202124;
const FG: u32 = 0xe8eaed;
const BTN: u32 = 0x3c4043;
const ACCENT: u32 = 0x8ab4f8;
const DIM: u32 = 0x5f6368;
const ERR: u32 = 0xf28b82;

fn btn(i: usize) -> Rect {
    Rect { x: PAD + i as i32 * (BTN_W + GAP), y: PAD, w: BTN_W, h: ROW_H }
}

fn vol_at(x: i32) -> i32 {
    (x - BAR.x) * 100 / BAR.w
}

fn gain(vol: i32) -> f32 {
    let x = vol as f32 / 100.0;
    x * x
}

struct Canvas<'a> {
    pixels: &'a mut [u32],
    width: i32,
    height: i32,
    scale: i32,
}

impl Canvas<'_> {
    fn rect(&mut self, Rect { x, y, w, h }: Rect, color: u32) {
        let (x0, y0) = ((x * self.scale).max(0), (y * self.scale).max(0));
        let (x1, y1) = (((x + w) * self.scale).min(self.width), ((y + h) * self.scale).min(self.height));
        if x0 >= x1 {
            return;
        }
        for yy in y0..y1 {
            let row = (yy * self.width) as usize;
            self.pixels[row + x0 as usize..row + x1 as usize].fill(color);
        }
    }

    fn text(&mut self, x: i32, y: i32, text: &str, color: u32) {
        for (i, ch) in text.bytes().enumerate() {
            let Some(glyph) = BASIC_LEGACY.get(ch as usize) else { continue };
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..8 {
                    if bits >> col & 1 == 1 {
                        self.rect(Rect { x: x + i as i32 * 8 + col, y: y + row as i32, w: 1, h: 1 }, color);
                    }
                }
            }
        }
    }

    fn label(&mut self, r: Rect, text: &str, color: u32) {
        self.text(r.x + (r.w - text.len() as i32 * 8) / 2, r.y + (r.h - 8) / 2, text, color);
    }
}

struct View {
    // Declared first so it drops before the window it draws on.
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    window: Rc<Window>,
}

impl View {
    fn open(el: &ActiveEventLoop) -> Result<View, String> {
        let attrs = Window::default_attributes()
            .with_title("noiseman")
            .with_inner_size(LogicalSize::new(WIN_W, WIN_H))
            .with_resizable(false);
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "noiseman", "noiseman");
        let window = Rc::new(el.create_window(attrs).map_err(|e| format!("cannot create window: {e}"))?);
        let surface = softbuffer::Context::new(window.clone())
            .and_then(|ctx| softbuffer::Surface::new(&ctx, window.clone()))
            .map_err(|e| format!("cannot create framebuffer: {e}"))?;
        Ok(View { surface, window })
    }

    fn scale(&self) -> i32 {
        let size = self.window.inner_size();
        (size.width as i32 / WIN_W).min(size.height as i32 / WIN_H).max(1)
    }
}

struct App {
    proxy: EventLoopProxy<Event>,
    tray: Option<tray::Tray>,
    view: Option<View>,
    stream: Option<audio::Output>,
    drop_stream_at: Option<Instant>,
    vol: i32,
    muted: bool,
    status: String,
    cursor: (i32, i32),
    dragging: bool,
}

impl App {
    fn play(&mut self) {
        let proxy = self.proxy.clone();
        let on_lost = move || send_or_log(&proxy, Event::AudioLost, "signal audio loss");
        match audio::start(on_lost) {
            Ok(s) => {
                self.stream = Some(s);
                self.status.clear();
            }
            Err(e) => {
                eprintln!("noiseman: cannot start audio: {e}");
                self.status = format!("Audio error: {e}");
                self.muted = true;
            }
        }
    }

    fn apply_gain(&self) {
        audio::set_gain(if self.muted { 0.0 } else { gain(self.vol) });
    }

    fn set_vol(&mut self, v: i32) {
        self.vol = v.clamp(0, 100);
        self.apply_gain();
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.apply_gain();
        if muted {
            let queued = self.stream.as_ref().map_or(Duration::ZERO, audio::Output::buffer_time);
            // 100 ms covers the 10 ms-τ fade-out; the buffers cover audio already queued in the device.
            self.drop_stream_at = Some(Instant::now() + Duration::from_millis(100) + 3 * queued);
        } else {
            self.drop_stream_at = None;
            if self.stream.is_none() {
                self.play();
            }
        }
    }

    fn click(&mut self, x: i32, y: i32) {
        if let Some(i) = (0..KINDS).find(|&i| btn(i).contains(x, y)) {
            audio::set_kind(Kind::ALL[i]);
        } else if BAR.contains(x, y) {
            self.dragging = true;
            self.set_vol(vol_at(x));
        } else if MUTE.contains(x, y) {
            self.set_muted(!self.muted);
        } else {
            return;
        }
        self.refresh();
    }

    fn tray_state(&self) -> TrayState {
        TrayState { shown: self.view.is_some(), muted: self.muted, kind: audio::kind() }
    }

    fn refresh(&mut self) {
        if let Some(v) = &self.view {
            v.window.request_redraw();
        }
        let state = self.tray_state();
        if let Some(t) = &mut self.tray {
            t.sync(state);
        }
    }

    fn hide(&mut self) {
        self.view = None;
        self.dragging = false;
    }

    fn show(&mut self, el: &ActiveEventLoop) {
        if self.view.is_some() {
            return;
        }
        match View::open(el) {
            Ok(v) => self.view = Some(v),
            Err(e) => {
                eprintln!("noiseman: {e}");
                el.exit();
            }
        }
    }

    fn draw(&mut self) -> Result<(), softbuffer::SoftBufferError> {
        let Some(view) = &mut self.view else { return Ok(()) };
        let scale = view.scale();
        let size = view.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return Ok(()) };
        view.surface.resize(w, h)?;
        let mut buf = view.surface.buffer_mut()?;
        buf.fill(BG);
        let mut c = Canvas { pixels: &mut buf, width: w.get() as i32, height: h.get() as i32, scale };
        let current = audio::kind();
        for (i, k) in Kind::ALL.iter().enumerate() {
            let (bg, fg) = if *k == current { (ACCENT, BG) } else { (BTN, FG) };
            c.rect(btn(i), bg);
            c.label(btn(i), k.label(), fg);
        }
        c.rect(BAR, BTN);
        c.rect(Rect { w: BAR.w * self.vol / 100, ..BAR }, if self.muted { DIM } else { ACCENT });
        c.text(BAR.x + BAR.w + 8, BAR.y + 6, &format!("{:>3}%", self.vol), FG);
        c.rect(Rect { x: MUTE.x, y: MUTE.y + 2, w: 16, h: 16 }, FG);
        c.rect(Rect { x: MUTE.x + 2, y: MUTE.y + 4, w: 12, h: 12 }, if self.muted { ACCENT } else { BG });
        c.text(MUTE.x + 24, MUTE.y + 6, "Mute", FG);
        c.text(PAD, STATUS_Y, &self.status, ERR);
        buf.present()
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        self.show(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Event) {
        match event {
            Event::AudioLost => {
                self.stream = None;
                if !self.muted {
                    self.play();
                }
            }
            Event::ToggleWindow if self.view.is_some() => self.hide(),
            Event::ToggleWindow => self.show(el),
            Event::ToggleMute => self.set_muted(!self.muted),
            Event::Quit => el.exit(),
            Event::TrayReady(t) => self.tray = Some(t),
            Event::TrayGone => {
                self.tray = None;
                self.show(el);
            }
        }
        self.refresh();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested if self.tray.is_some() => {
                self.hide();
                self.refresh();
            }
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw() {
                    eprintln!("noiseman: redraw failed: {e}");
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let s = self.view.as_ref().map_or(1, View::scale);
                self.cursor = (position.x as i32 / s, position.y as i32 / s);
                if self.dragging {
                    self.set_vol(vol_at(self.cursor.0));
                    self.refresh();
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => self.click(self.cursor.0, self.cursor.1),
                ElementState::Released => self.dragging = false,
            },
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key, state: ElementState::Pressed, repeat, .. }, .. } => {
                match logical_key.as_ref() {
                    Key::Named(NamedKey::ArrowLeft) => self.set_vol(self.vol - 5),
                    Key::Named(NamedKey::ArrowRight) => self.set_vol(self.vol + 5),
                    Key::Character("m") | Key::Named(NamedKey::Space) if !repeat => self.set_muted(!self.muted),
                    Key::Character(c) => match c.parse::<usize>() {
                        Ok(n @ 1..=KINDS) => audio::set_kind(Kind::ALL[n - 1]),
                        _ => return,
                    },
                    _ => return,
                }
                self.refresh();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        match self.drop_stream_at {
            Some(t) if Instant::now() >= t => {
                self.stream = None;
                self.drop_stream_at = None;
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }
}

fn main() {
    let event_loop = match EventLoop::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("noiseman: cannot start event loop: {e}");
            std::process::exit(1);
        }
    };
    let mut app = App {
        proxy: event_loop.create_proxy(),
        tray: None,
        view: None,
        stream: None,
        drop_stream_at: None,
        vol: 50,
        muted: false,
        status: String::new(),
        cursor: (0, 0),
        dragging: false,
    };
    app.set_muted(false);
    // The window is created later, in `resumed`.
    tray::spawn(app.proxy.clone(), TrayState { shown: true, ..app.tray_state() });
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("noiseman: event loop failed: {e}");
        std::process::exit(1);
    }
}
