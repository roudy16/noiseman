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
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use cpal::traits::StreamTrait;
use font8x8::legacy::BASIC_LEGACY;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use audio::{BUFFER_FRAMES, GAIN, KIND};
use noise::Kind;

type Rect = (i32, i32, i32, i32);
type TrayState = (bool, bool, Kind);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Event {
    AudioLost,
    Toggle,
    Mute,
    Quit,
    TrayGone,
    TrayReady(tray::Tray),
}

const PAD: i32 = 8;
const BTN_W: i32 = 56;
const ROW_H: i32 = 20;
const WIN_W: i32 = PAD * 2 + 5 * (BTN_W + 4) - 4;
const WIN_H: i32 = 124;
const BAR: Rect = (PAD, 40, 240, ROW_H);
const MUTE: Rect = (PAD, 72, 64, ROW_H);
const BG: u32 = 0x202124;
const FG: u32 = 0xe8eaed;
const BTN: u32 = 0x3c4043;
const ACCENT: u32 = 0x8ab4f8;

fn btn(i: usize) -> Rect {
    (PAD + i as i32 * (BTN_W + 4), 8, BTN_W, ROW_H)
}

fn hit((x, y, w, h): Rect, px: i32, py: i32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn gain(vol: i32) -> f32 {
    let x = vol as f32 / 100.0;
    x * x
}

struct Canvas<'a> {
    px: &'a mut [u32],
    w: i32,
    h: i32,
    s: i32,
}

impl Canvas<'_> {
    fn rect(&mut self, (x, y, w, h): Rect, c: u32) {
        let (x0, y0) = ((x * self.s).max(0), (y * self.s).max(0));
        let (x1, y1) = (((x + w) * self.s).min(self.w), ((y + h) * self.s).min(self.h));
        if x0 >= x1 {
            return;
        }
        for yy in y0..y1 {
            let row = (yy * self.w) as usize;
            self.px[row + x0 as usize..row + x1 as usize].fill(c);
        }
    }

    fn text(&mut self, x: i32, y: i32, s: &str, c: u32) {
        for (i, ch) in s.bytes().enumerate() {
            let Some(glyph) = BASIC_LEGACY.get(ch as usize) else { continue };
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..8 {
                    if bits >> col & 1 == 1 {
                        self.rect((x + i as i32 * 8 + col, y + row as i32, 1, 1), c);
                    }
                }
            }
        }
    }

    fn label(&mut self, (x, y, w, h): Rect, s: &str, c: u32) {
        self.text(x + (w - s.len() as i32 * 8) / 2, y + (h - 8) / 2, s, c);
    }
}

struct App {
    proxy: EventLoopProxy<Event>,
    tray: Option<tray::Tray>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    stream: Option<(cpal::Stream, cpal::SampleRate)>,
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
        let on_lost = move || {
            if let Err(e) = proxy.send_event(Event::AudioLost) {
                eprintln!("noiseman: cannot signal audio loss, event loop gone: {e}");
            }
        };
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

    fn set_vol(&mut self, v: i32) {
        self.vol = v.clamp(0, 100);
        if !self.muted {
            GAIN.store(gain(self.vol).to_bits(), Relaxed);
        }
    }

    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if self.muted {
            GAIN.store(0, Relaxed);
            let buffer_ms = self.stream.as_ref().map_or(0, |(s, rate)| {
                let frames = s.buffer_size().unwrap_or_else(|e| {
                    eprintln!("noiseman: stream buffer size unavailable, assuming {BUFFER_FRAMES}: {e}");
                    BUFFER_FRAMES
                });
                u64::from(frames) * 1000 / u64::from(*rate)
            });
            self.drop_stream_at = Some(Instant::now() + Duration::from_millis(100 + 3 * buffer_ms));
        } else {
            self.drop_stream_at = None;
            GAIN.store(gain(self.vol).to_bits(), Relaxed);
            if self.stream.is_none() {
                self.play();
            }
        }
    }

    fn scale(&self) -> i32 {
        self.window.as_ref().map_or(1, |w| {
            let size = w.inner_size();
            (size.width as i32 / WIN_W).min(size.height as i32 / WIN_H).max(1)
        })
    }

    fn click(&mut self, x: i32, y: i32) {
        if let Some(i) = (0..Kind::ALL.len()).find(|&i| hit(btn(i), x, y)) {
            KIND.store(i as u8, Relaxed);
        } else if hit(BAR, x, y) {
            self.dragging = true;
            self.set_vol((x - BAR.0) * 100 / BAR.2);
        } else if hit(MUTE, x, y) {
            self.toggle_mute();
        } else {
            return;
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        let state = (self.window.is_some(), self.muted, Kind::from_u8(KIND.load(Relaxed)));
        if let Some(t) = &mut self.tray {
            t.sync(state);
        }
    }

    fn hide(&mut self) {
        self.surface = None;
        self.window = None;
        self.dragging = false;
    }

    fn show(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("noiseman")
            .with_inner_size(LogicalSize::new(WIN_W, WIN_H))
            .with_resizable(false);
        #[cfg(all(unix, not(target_vendor = "apple")))]
        let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "noiseman", "noiseman");
        let window = match el.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("noiseman: cannot create window: {e}");
                el.exit();
                return;
            }
        };
        let surface = softbuffer::Context::new(window.clone())
            .and_then(|ctx| softbuffer::Surface::new(&ctx, window.clone()));
        match surface {
            Ok(s) => self.surface = Some(s),
            Err(e) => {
                eprintln!("noiseman: cannot create framebuffer: {e}");
                el.exit();
                return;
            }
        }
        self.window = Some(window);
    }

    fn draw(&mut self) -> Result<(), softbuffer::SoftBufferError> {
        let s = self.scale();
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else { return Ok(()) };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return Ok(()) };
        surface.resize(w, h)?;
        let mut buf = surface.buffer_mut()?;
        buf.fill(BG);
        let mut c = Canvas { px: &mut buf, w: w.get() as i32, h: h.get() as i32, s };
        let cur = KIND.load(Relaxed) as usize;
        for (i, k) in Kind::ALL.iter().enumerate() {
            let (bg, fg) = if i == cur { (ACCENT, BG) } else { (BTN, FG) };
            c.rect(btn(i), bg);
            c.label(btn(i), k.label(), fg);
        }
        c.rect(BAR, BTN);
        c.rect((BAR.0, BAR.1, BAR.2 * self.vol / 100, BAR.3), if self.muted { 0x5f6368 } else { ACCENT });
        c.text(BAR.0 + BAR.2 + 8, BAR.1 + 6, &format!("{:>3}%", self.vol), FG);
        c.rect((MUTE.0, MUTE.1 + 2, 16, 16), FG);
        c.rect((MUTE.0 + 2, MUTE.1 + 4, 12, 12), if self.muted { ACCENT } else { BG });
        c.text(MUTE.0 + 24, MUTE.1 + 6, "Mute", FG);
        c.text(PAD, 104, &self.status, 0xf28b82);
        buf.present()
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_none() {
            self.show(el);
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Event) {
        match event {
            Event::AudioLost => {
                self.stream = None;
                if !self.muted {
                    self.play();
                }
            }
            Event::Toggle if self.window.is_some() => self.hide(),
            Event::Toggle => self.show(el),
            Event::Mute => self.toggle_mute(),
            Event::Quit => el.exit(),
            Event::TrayReady(t) => self.tray = Some(t),
            Event::TrayGone => {
                self.tray = None;
                if self.window.is_none() {
                    self.show(el);
                }
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
                let s = self.scale();
                self.cursor = (position.x as i32 / s, position.y as i32 / s);
                if self.dragging {
                    self.set_vol((self.cursor.0 - BAR.0) * 100 / BAR.2);
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
                    Key::Character("m") | Key::Named(NamedKey::Space) if !repeat => self.toggle_mute(),
                    Key::Character(c) => match c.parse::<usize>() {
                        Ok(n @ 1..=5) => KIND.store(n as u8 - 1, Relaxed),
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
        window: None,
        surface: None,
        stream: None,
        drop_stream_at: None,
        vol: 50,
        muted: false,
        status: String::new(),
        cursor: (0, 0),
        dragging: false,
    };
    if !app.muted {
        GAIN.store(gain(app.vol).to_bits(), Relaxed);
        app.play();
    }
    tray::spawn(app.proxy.clone(), (true, app.muted, Kind::from_u8(KIND.load(Relaxed))));
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("noiseman: event loop failed: {e}");
        std::process::exit(1);
    }
}
