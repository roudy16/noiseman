#[cfg(target_os = "linux")]
mod tray;
#[cfg(not(target_os = "linux"))]
mod tray {
    pub enum Tray {}

    pub fn spawn(_: winit::event_loop::EventLoopProxy<super::Event>, _: super::TrayState) {}

    impl Tray {
        pub fn sync(&mut self, _: super::TrayState) {
            match *self {}
        }
    }
}

use std::num::NonZeroU32;
use std::process::ExitCode;
use std::rc::Rc;
use std::thread;

use font8x8::legacy::BASIC_LEGACY;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use super::client::{Client, Sender};
use crate::platform;
use crate::protocol::{Command, Kind, State};

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum Event {
    Reply(Result<State, String>),
    PlayerGone(Option<String>),
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

/// An error that ends the window, also shown in a message box where stderr goes nowhere (`noiseman-gui.exe`).
fn fail(e: &str) {
    eprintln!("noiseman: {e}");
    platform::alert_if_no_console(e);
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
const WIN_W: i32 = 328;
const MARGIN: i32 = 8;
const IN_X: i32 = MARGIN + 11;
const IN_W: i32 = WIN_W - 2 * IN_X;
const LOGO: Rect = Rect { x: IN_X, y: MARGIN + 9, w: 63, h: 24 };
const KINDS_Y: i32 = LOGO.y + LOGO.h + 8;
const VOL_Y: i32 = KINDS_Y + 16;
const BAR: Rect = Rect { x: IN_X, y: VOL_Y + 2, w: IN_W - 40, h: 4 };
const TOGGLE: Rect = Rect { x: IN_X, y: VOL_Y + 16, w: IN_W, h: 16 };
const CARD: Rect = Rect { x: MARGIN, y: MARGIN, w: WIN_W - 2 * MARGIN, h: TOGGLE.y + TOGGLE.h + 9 - MARGIN };
const ERR_H: i32 = 16;
const WIN_H: i32 = CARD.y + CARD.h + ERR_H + 24;
const BG: u32 = 0x1d1d1d;
const FG: u32 = 0xdcdcdc;
const DIM: u32 = 0x8b8b8b;
const GREEN: u32 = 0x4cae5a;
const RED: u32 = 0xd94f45;
/// tui.rs LOGO as bar heights in sixteenths of the tile.
const LOGO_BARS: [i32; 12] = [5, 11, 7, 14, 9, 3, 12, 6, 15, 8, 4, 10];

// Glyphs font8x8 lacks, in its layout: one byte per row, bit 0 is the leftmost pixel.
const DOT: [u8; 8] = [0x00, 0x3C, 0x7E, 0x7E, 0x7E, 0x7E, 0x3C, 0x00];
const RING: [u8; 8] = [0x00, 0x14, 0x40, 0x02, 0x40, 0x02, 0x28, 0x00];
const CROSS: [u8; 8] = [0x00, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x00];
const MIDDOT: [u8; 8] = [0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00];
const LEFT: [u8; 8] = [0x00, 0x04, 0x06, 0x3F, 0x3F, 0x06, 0x04, 0x00];
const RIGHT: [u8; 8] = [0x00, 0x08, 0x18, 0x3F, 0x3F, 0x18, 0x08, 0x00];

fn glyph(c: char) -> Option<&'static [u8; 8]> {
    match c {
        '●' => Some(&DOT),
        '◌' => Some(&RING),
        '✕' => Some(&CROSS),
        '·' => Some(&MIDDOT),
        '←' => Some(&LEFT),
        '→' => Some(&RIGHT),
        _ => BASIC_LEGACY.get(c as usize),
    }
}

/// Spaces are half width, so the key hints fit a small window.
fn advance(c: char) -> i32 {
    if c == ' ' { 4 } else { 8 }
}

fn text_w(s: &str) -> i32 {
    s.chars().map(advance).sum()
}

struct Theme {
    accent: u32,
    track: u32,
    /// The volume bar past the level: the track, dimmed as the TUI does.
    rest: u32,
    tile: u32,
}

/// Mixes `a` with `b`, `pct` percent of `a`, per channel.
fn mix(a: u32, b: u32, pct: u32) -> u32 {
    (0..3).map(|i| (((a >> (i * 8) & 0xff) * pct + (b >> (i * 8) & 0xff) * (100 - pct)) / 100) << (i * 8)).sum()
}

fn theme(kind: Kind) -> Theme {
    let accent = match kind {
        Kind::White => 0x8c96a8,
        Kind::Pink => 0xd4708f,
        Kind::Brown => 0xd08f4a,
        Kind::Blue => 0x4f8fd6,
        Kind::Violet => 0x9b74d6,
    };
    let track = mix(accent, 0x8a8a8a, 35);
    Theme { accent, track, rest: mix(track, BG, 50), tile: mix(accent, 0, 22) }
}

/// The error line pushes the card's bottom down; without it the layout is centered in the same window.
fn shift(error: bool) -> i32 {
    if error { 0 } else { ERR_H / 2 }
}

/// Left edge and width of kind `i`'s "N Label", spread across the card.
fn kind_span(i: usize) -> (i32, i32) {
    let w = |k: &Kind| 12 + text_w(k.label());
    let total: i32 = Kind::ALL.iter().map(w).sum();
    let before: i32 = Kind::ALL[..i].iter().map(w).sum();
    (IN_X + before + (IN_W - total) * i as i32 / (KINDS as i32 - 1), w(&Kind::ALL[i]))
}

fn kind_hit(i: usize) -> Rect {
    let (x, w) = kind_span(i);
    Rect { x: x - 4, y: KINDS_Y - 4, w: w + 8, h: 16 }
}

fn bar_hit() -> Rect {
    Rect { y: VOL_Y - 4, h: 16, ..BAR }
}

fn vol_at(x: i32) -> i32 {
    (x - BAR.x) * 100 / BAR.w
}

struct Canvas<'a> {
    pixels: &'a mut [u32],
    width: i32,
    height: i32,
    scale: i32,
    dy: i32,
}

impl Canvas<'_> {
    fn rect(&mut self, Rect { x, y, w, h }: Rect, color: u32) {
        let y = y + self.dy;
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

    /// A 1px outline with the corner pixels left out, for a hint of rounding.
    fn frame(&mut self, r: Rect, color: u32) {
        self.rect(Rect { x: r.x + 1, w: r.w - 2, h: 1, ..r }, color);
        self.rect(Rect { x: r.x + 1, y: r.y + r.h - 1, w: r.w - 2, h: 1 }, color);
        self.rect(Rect { y: r.y + 1, w: 1, h: r.h - 2, ..r }, color);
        self.rect(Rect { x: r.x + r.w - 1, y: r.y + 1, w: 1, h: r.h - 2 }, color);
    }

    fn fill(&mut self, r: Rect, color: u32) {
        self.rect(Rect { x: r.x + 1, w: r.w - 2, ..r }, color);
        self.rect(Rect { y: r.y + 1, h: r.h - 2, ..r }, color);
    }

    /// Bold widens strokes one pixel right, except into a gap whose right side is lit (keeps "m" open).
    /// Returns the x after the text.
    fn text(&mut self, mut x: i32, y: i32, text: &str, color: u32, bold: bool) -> i32 {
        for ch in text.chars() {
            for (row, &bits) in glyph(ch).into_iter().flatten().enumerate() {
                let bits = if bold { bits | (bits << 1 & !(bits >> 1)) } else { bits };
                for col in 0..8 {
                    if bits >> col & 1 == 1 {
                        self.rect(Rect { x: x + col, y: y + row as i32, w: 1, h: 1 }, color);
                    }
                }
            }
            x += advance(ch);
        }
        x
    }
}

/// Paints the whole window at an integer scale into a `width` x `height` buffer.
fn paint(pixels: &mut [u32], width: i32, height: i32, scale: i32, s: &State, tray: bool, error: Option<&str>) {
    pixels.fill(BG);
    let t = theme(s.kind);
    let mut c = Canvas { pixels, width, height, scale, dy: shift(error.is_some()) };
    let card = Rect { h: CARD.h + if error.is_some() { ERR_H } else { 0 }, ..CARD };
    let right = IN_X + IN_W;
    let level = if s.muted { t.track } else { t.accent };
    c.frame(card, t.track);
    c.rect(LOGO, t.tile);
    for (i, h) in LOGO_BARS.iter().enumerate() {
        let h = h * LOGO.h / 16;
        c.rect(Rect { x: LOGO.x + 2 + i as i32 * 5, y: LOGO.y + LOGO.h - h, w: 4, h }, level);
    }
    let x = LOGO.x + LOGO.w + 10;
    c.text(x, LOGO.y + 2, "noiseman", t.accent, true);
    c.text(x, LOGO.y + 14, "noise generator", DIM, false);
    if s.muted {
        c.text(right - text_w("◌ Muted"), LOGO.y + 2, "◌ Muted", DIM, false);
    } else {
        let x = c.text(right - text_w("● Playing"), LOGO.y + 2, "●", GREEN, false);
        c.text(x, LOGO.y + 2, " Playing", FG, false);
    }
    for (i, k) in Kind::ALL.iter().enumerate() {
        let selected = *k == s.kind;
        let x = c.text(kind_span(i).0, KINDS_Y, &"12345"[i..=i], if selected { t.accent } else { t.track }, false);
        c.text(x + 4, KINDS_Y, k.label(), if selected { t.accent } else { FG }, selected);
    }
    let vol = s.vol.clamp(0, 100);
    c.rect(BAR, t.rest);
    c.rect(Rect { w: BAR.w * vol / 100, ..BAR }, level);
    let mut digits = *b"100%";
    let n = 2 - (vol >= 10) as usize - (vol == 100) as usize;
    digits[1] = b'0' + (vol / 10 % 10) as u8;
    digits[2] = b'0' + (vol % 10) as u8;
    let pct = std::str::from_utf8(&digits[n..]).unwrap_or("?");
    c.text(right - text_w(pct), VOL_Y, pct, FG, false);
    let (label, ink) = if s.muted {
        c.fill(TOGGLE, t.accent);
        ("Play", BG)
    } else {
        c.frame(TOGGLE, t.accent);
        ("Mute", t.accent)
    };
    c.text(TOGGLE.x + (TOGGLE.w - text_w(label)) / 2, TOGGLE.y + 4, label, ink, true);
    if let Some(e) = error {
        let x = c.text(IN_X, TOGGLE.y + TOGGLE.h + 8, "✕ ", RED, false);
        let e = e.char_indices().nth(((right - x) / 8) as usize).map_or(e, |(i, _)| &e[..i]);
        c.text(x, TOGGLE.y + TOGGLE.h + 8, e, RED, false);
    }
    let hints =
        [("1-5", "noise"), ("←→", "vol"), ("m", "mute"), ("q", if tray { "hide" } else { "quit" }), ("s", "stop")];
    let sep = " · ";
    let total = hints.iter().map(|(k, d)| text_w(k) + 4 + text_w(d)).sum::<i32>() + 4 * text_w(sep);
    let (mut x, y) = ((WIN_W - total) / 2, card.y + card.h + 8);
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            x = c.text(x, y, sep, DIM, false);
        }
        x = c.text(x, y, key, t.accent, true);
        x = c.text(x + 4, y, desc, DIM, false);
    }
}

/// The whole scale nearest the OS's: 2.5 (Windows 250%) draws at 3, and 1.25 at 1 rather than 2, a window 60% larger
/// than asked.
fn whole_scale(scale_factor: f64) -> i32 {
    (scale_factor.round() as i32).max(1)
}

/// The window's size in physical pixels, so paint() fills it at a whole scale with no band left over.
fn window_size(scale_factor: f64) -> PhysicalSize<u32> {
    let s = whole_scale(scale_factor) as u32;
    PhysicalSize::new(WIN_W as u32 * s, WIN_H as u32 * s)
}

/// On X11 and Wayland winit pins a fixed-size window with min = max = its size, which the window manager enforces, so
/// the limits move with it. Windows has no such limits, and setting them during a DPI change would resize the window
/// to its old size under winit's own resize.
#[cfg_attr(not(all(unix, not(target_vendor = "apple"))), allow(unused_variables))]
fn set_size_limits(window: &Window, size: PhysicalSize<u32>) {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        window.set_min_inner_size(Some(size));
        window.set_max_inner_size(Some(size));
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
        // The icon build.rs embeds as resource 1: title bar (small) and taskbar / Alt-Tab (large).
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::{IconExtWindows, WindowAttributesExtWindows};
            let icon = || winit::window::Icon::from_resource(1, None).ok();
            attrs.with_window_icon(icon()).with_taskbar_icon(icon())
        };
        let window = Rc::new(el.create_window(attrs).map_err(|e| format!("cannot create window: {e}"))?);
        let surface = softbuffer::Context::new(window.clone())
            .and_then(|ctx| softbuffer::Surface::new(&ctx, window.clone()))
            .map_err(|e| format!("cannot create framebuffer: {e}"))?;
        let size = window_size(window.scale_factor());
        set_size_limits(&window, size);
        let _ = window.request_inner_size(size);
        Ok(View { surface, window })
    }

    /// Wayland sizes windows in whole logical pixels, so at a fractional scale the window can come out up to `s`
    /// pixels short of `window_size`; paint() clips that much of the margin. A window kept smaller draws smaller.
    fn scale(&self) -> i32 {
        let size = self.window.inner_size();
        let s = whole_scale(self.window.scale_factor());
        ((size.width as i32 + s) / WIN_W).min((size.height as i32 + s) / WIN_H).max(1)
    }
}

struct App {
    proxy: EventLoopProxy<Event>,
    sender: Sender,
    tray: Option<tray::Tray>,
    view: Option<View>,
    state: State,
    problem: Option<String>,
    warning: Option<String>,
    cursor: (i32, i32),
    mods: ModifiersState,
    dragging: bool,
    failed: bool,
}

impl App {
    fn send(&mut self, cmd: Command) {
        if let Err(e) = self.sender.send(cmd) {
            eprintln!("noiseman: {e}");
            self.problem = Some(e);
        }
    }

    /// Shown at once so a volume drag does not wait for the player's reply.
    fn set_vol(&mut self, v: i32) {
        self.state.vol = v.clamp(0, 100);
        self.send(Command::Vol(self.state.vol));
    }

    /// Stops the player too: with no tray left to bring the window back, quitting means silence.
    fn quit(&mut self, el: &ActiveEventLoop) {
        self.send(Command::Quit);
        el.exit();
    }

    /// The close button, `q` or Esc: hide to the tray, or quit when there is none.
    fn close(&mut self, el: &ActiveEventLoop) {
        if self.tray.is_some() {
            self.hide();
            self.refresh();
        } else {
            self.quit(el);
        }
    }

    fn error(&self) -> Option<&str> {
        self.problem.as_deref().or(self.state.error.as_deref()).or(self.warning.as_deref())
    }

    fn click(&mut self, x: i32, y: i32) {
        let y = y - shift(self.error().is_some());
        if let Some(i) = (0..KINDS).find(|&i| kind_hit(i).contains(x, y)) {
            self.send(Command::Kind(Kind::ALL[i]));
        } else if bar_hit().contains(x, y) {
            self.dragging = true;
            self.set_vol(vol_at(x));
        } else if TOGGLE.contains(x, y) {
            self.send(Command::ToggleMute);
        } else {
            return;
        }
        self.refresh();
    }

    fn tray_state(&self) -> TrayState {
        TrayState { shown: self.view.is_some(), muted: self.state.muted, kind: self.state.kind }
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
                fail(&e);
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
        let error = self.problem.as_deref().or(self.state.error.as_deref()).or(self.warning.as_deref());
        paint(&mut buf, w.get() as i32, h.get() as i32, scale, &self.state, self.tray.is_some(), error);
        buf.present()
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        self.show(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Event) {
        match event {
            Event::Reply(Ok(s)) => {
                // While dragging, the bar shows the pointer, not replies to earlier positions.
                let vol = self.state.vol;
                self.state = s;
                if self.dragging {
                    self.state.vol = vol;
                }
                self.problem = None;
            }
            Event::Reply(Err(e)) => self.problem = Some(e),
            Event::PlayerGone(why) => {
                if let Some(e) = why {
                    fail(&e);
                    self.failed = true;
                }
                el.exit();
            }
            Event::ToggleWindow if self.view.is_some() => self.hide(),
            Event::ToggleWindow => self.show(el),
            Event::ToggleMute => self.send(Command::ToggleMute),
            Event::Quit => self.quit(el),
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
            WindowEvent::CloseRequested => self.close(el),
            WindowEvent::ScaleFactorChanged { scale_factor, mut inner_size_writer } => {
                let size = window_size(scale_factor);
                if let Some(v) = &self.view {
                    set_size_limits(&v.window, size);
                }
                if let Err(e) = inner_size_writer.request_inner_size(size) {
                    eprintln!("noiseman: cannot resize window for scale {scale_factor}: {e}");
                }
            }
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
                ElementState::Released if self.dragging => {
                    self.dragging = false;
                    // Replies were held back during the drag; fetch the real state now.
                    self.send(Command::Status);
                }
                ElementState::Released => {}
            },
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput {
                event: KeyEvent { logical_key, state: ElementState::Pressed, repeat, .. },
                ..
            } if !self.mods.intersects(ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SUPER) => {
                let cycle = |step: usize| {
                    let i = Kind::ALL.iter().position(|&k| k == self.state.kind).unwrap_or(0);
                    Command::Kind(Kind::ALL[(i + step) % KINDS])
                };
                let (next, prev) = (cycle(1), cycle(KINDS - 1));
                match logical_key.as_ref() {
                    Key::Character("q") | Key::Named(NamedKey::Escape) if !repeat => return self.close(el),
                    Key::Character("s") if !repeat => return self.quit(el),
                    Key::Named(NamedKey::ArrowLeft) | Key::Character("h" | "-") => self.send(Command::VolBy(-5)),
                    Key::Named(NamedKey::ArrowRight) | Key::Character("l" | "+" | "=") => self.send(Command::VolBy(5)),
                    Key::Named(NamedKey::Tab) if self.mods.shift_key() => self.send(prev),
                    Key::Named(NamedKey::ArrowUp | NamedKey::Tab) | Key::Character("k") => self.send(next),
                    Key::Named(NamedKey::ArrowDown) | Key::Character("j") => self.send(prev),
                    Key::Character("m") | Key::Named(NamedKey::Space) if !repeat => self.send(Command::ToggleMute),
                    Key::Character(c) => match c.parse::<usize>() {
                        Ok(n @ 1..=KINDS) => self.send(Command::Kind(Kind::ALL[n - 1])),
                        _ => return,
                    },
                    _ => return,
                }
                self.refresh();
            }
            _ => {}
        }
    }
}

pub fn run() -> ExitCode {
    let event_loop = match EventLoop::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            fail(&format!("cannot start event loop: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let proxy = event_loop.create_proxy();
    let Client { mut sender, mut receiver, warning, .. } = match Client::connect(true) {
        Ok(c) => c,
        Err(e) => {
            fail(&e);
            return ExitCode::FAILURE;
        }
    };
    let first = match sender.send(Command::Watch).and_then(|()| receiver.recv()) {
        Ok(Some(Ok(s))) => s,
        Ok(Some(Err(e))) | Err(e) => {
            fail(&e);
            return ExitCode::FAILURE;
        }
        Ok(None) => {
            fail("player stopped");
            return ExitCode::FAILURE;
        }
    };
    let replies = proxy.clone();
    thread::spawn(move || {
        let gone = loop {
            match receiver.recv() {
                Ok(Some(r)) => send_or_log(&replies, Event::Reply(r), "forward player state"),
                Ok(None) => break None,
                Err(e) => break Some(e),
            }
        };
        // After a quit from this window the event loop is already gone, and nobody needs telling.
        let _ = replies.send_event(Event::PlayerGone(gone));
    });
    let mut app = App {
        proxy,
        sender,
        tray: None,
        view: None,
        state: first,
        problem: None,
        warning,
        cursor: (0, 0),
        mods: ModifiersState::empty(),
        dragging: false,
        failed: false,
    };
    // The window is created later, in `resumed`.
    tray::spawn(app.proxy.clone(), TrayState { shown: true, ..app.tray_state() });
    if let Err(e) = event_loop.run_app(&mut app) {
        fail(&format!("event loop failed: {e}"));
        return ExitCode::FAILURE;
    }
    if app.failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_each_state() {
        let state = |kind, vol, muted, error: Option<&str>| State { kind, vol, muted, error: error.map(String::from) };
        let frames = [
            ("brown-playing", state(Kind::Brown, 50, false, None), true),
            ("pink-muted", state(Kind::Pink, 30, true, None), false),
            ("blue-error", state(Kind::Blue, 75, false, Some("audio device lost")), true),
        ];
        for scale in [2, 3] {
            let (w, h) = (WIN_W * scale, WIN_H * scale);
            let mut px = vec![0; (w * h) as usize];
            for (name, s, tray) in &frames {
                paint(&mut px, w, h, scale, s, *tray, s.error.as_deref());
                assert!(px.contains(&theme(s.kind).accent), "{name}@{scale}x: no accent");
                assert!(px.contains(&if s.error.is_some() { RED } else { FG }), "{name}@{scale}x: no text");
                let card_right = ((CARD.x + CARD.w) * scale - 1) as usize;
                assert_eq!(px[(CARD.y + 20) as usize * scale as usize * w as usize + card_right], theme(s.kind).track);
                let Some(dir) = std::env::var_os("NOISEMAN_GUI_SNAPSHOT") else { continue };
                let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
                ppm.extend(px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]));
                let file = std::path::Path::new(&dir).join(format!("gui-{name}-{scale}x.ppm"));
                std::fs::write(file, ppm).expect("write snapshot");
            }
        }
    }

    #[test]
    fn whole_scale_is_nearest() {
        for (factor, scale) in [(0.0, 1), (f64::NAN, 1), (1.0, 1), (1.25, 1), (1.5, 2), (1.75, 2), (2.0, 2), (2.25, 2)]
        {
            assert_eq!(whole_scale(factor), scale, "{factor}");
        }
        assert_eq!(whole_scale(2.5), 3);
        assert_eq!(window_size(2.5), PhysicalSize::new(WIN_W as u32 * 3, WIN_H as u32 * 3));
    }
}
