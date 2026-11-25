use std::io::{self, Stdout, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{
    self, Attribute, Color, Colored, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{
    self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};

use super::client::{Client, Sender};
use crate::platform;
use crate::protocol::{Command, Kind, State};

enum Ui {
    Input(Event),
    Reply(Result<State, String>),
    Gone(Option<String>),
}

/// Raw mode and the alternate screen, undone on drop and on panic.
struct Screen(Stdout);

impl Screen {
    fn enter() -> io::Result<Screen> {
        terminal::enable_raw_mode()?;
        let mut screen = Screen(io::stdout());
        execute!(screen.0, EnterAlternateScreen, Hide)?;
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            hook(info);
        }));
        Ok(screen)
    }
}

fn restore() {
    if let Err(e) = execute!(io::stdout(), Show, LeaveAlternateScreen).and_then(|()| terminal::disable_raw_mode()) {
        eprintln!("noiseman: cannot restore terminal: {e}");
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        restore();
    }
}

pub fn run() -> ExitCode {
    match exec() {
        Ok(Some(msg)) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("noiseman: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Returns a message to print after leaving the screen.
fn exec() -> Result<Option<String>, String> {
    let Client { mut sender, mut receiver } = Client::connect(true)?;
    sender.send(Command::Watch)?;
    let (tx, rx) = mpsc::channel();
    let replies = tx.clone();
    thread::spawn(move || {
        let gone = loop {
            match receiver.recv() {
                Ok(Some(r)) => {
                    if replies.send(Ui::Reply(r)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(e) => break Some(e),
            }
        };
        // The UI may already have quit; then nobody needs to hear about it.
        let _ = replies.send(Ui::Gone(gone));
    });
    thread::spawn(move || {
        loop {
            match event::read() {
                Ok(ev) => {
                    if tx.send(Ui::Input(ev)).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Ui::Gone(Some(format!("cannot read terminal: {e}"))));
                    return;
                }
            }
        }
    });

    let palette = Palette::detect();
    let mut screen = Screen::enter().map_err(|e| format!("cannot set up terminal: {e}"))?;
    let mut state: Option<State> = None;
    let mut problem: Option<String> = None;
    for msg in rx {
        match msg {
            Ui::Reply(Ok(s)) => {
                state = Some(s);
                problem = None;
            }
            Ui::Reply(Err(e)) => problem = Some(e),
            Ui::Gone(None) => return Ok(Some("player stopped".into())),
            Ui::Gone(Some(e)) => return Err(e),
            Ui::Input(Event::Key(key)) => match action(key, state.as_ref().map(|s| s.kind)) {
                Action::None => continue,
                Action::Leave => return Ok(None),
                Action::Send(cmd) => request(&mut sender, cmd, &mut problem),
                Action::Stop => {
                    request(&mut sender, Command::Quit, &mut problem);
                    return Ok(Some("player stopped".into()));
                }
            },
            Ui::Input(Event::Resize(..)) => {}
            Ui::Input(_) => continue,
        }
        draw(&mut screen.0, &palette, state.as_ref(), problem.as_deref()).map_err(|e| format!("cannot draw: {e}"))?;
    }
    Ok(None)
}

fn request(sender: &mut Sender, cmd: Command, problem: &mut Option<String>) {
    if let Err(e) = sender.send(cmd) {
        *problem = Some(e);
    }
}

enum Action {
    None,
    Leave,
    Stop,
    Send(Command),
}

fn action(key: KeyEvent, kind: Option<Kind>) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    let cycle = |step: usize| match kind.and_then(|k| Kind::ALL.iter().position(|&a| a == k)) {
        Some(i) => Action::Send(Command::Kind(Kind::ALL[(i + step) % Kind::ALL.len()])),
        None => Action::None,
    };
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Leave,
        _ if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => Action::None,
        KeyCode::Char('q') | KeyCode::Esc => Action::Leave,
        KeyCode::Char('s') => Action::Stop,
        KeyCode::Char('m' | ' ') => Action::Send(Command::ToggleMute),
        KeyCode::Left | KeyCode::Char('-' | 'h') => Action::Send(Command::VolBy(-5)),
        KeyCode::Right | KeyCode::Char('+' | '=' | 'l') => Action::Send(Command::VolBy(5)),
        KeyCode::Char(c @ '1'..='5') => Action::Send(Command::Kind(Kind::ALL[c as usize - '1' as usize])),
        KeyCode::Down | KeyCode::BackTab | KeyCode::Char('j') => cycle(Kind::ALL.len() - 1),
        KeyCode::Up | KeyCode::Tab | KeyCode::Char('k') => cycle(1),
        _ => Action::None,
    }
}

/// The colors for the terminal's depth: truecolor as is, 256 colors via the xterm cube, else none
/// (crossterm then drops every color sequence, and the layout leans on glyphs and bold/underline).
struct Palette {
    mono: bool,
    depth: u16,
}

/// The current noise kind's colors: its accent, a muted track and a dark logo tile.
struct Theme {
    accent: Color,
    track: Color,
    tile: Color,
}

impl Palette {
    fn detect() -> Palette {
        let var = |name| std::env::var(name).unwrap_or_default();
        let (colorterm, term) = (var("COLORTERM"), var("TERM"));
        let truecolor =
            colorterm.contains("truecolor") || colorterm.contains("24bit") || platform::terminal_truecolor();
        let depth = match (truecolor, term.contains("256")) {
            (true, _) => u16::MAX,
            (false, true) => 256,
            (false, false) => 8,
        };
        let mono = depth < 256 || Colored::ansi_color_disabled();
        if mono {
            style::force_color_output(false);
        }
        Palette { mono, depth }
    }

    fn rgb(&self, [r, g, b]: [u8; 3]) -> Color {
        if self.depth == 256 { cube(r, g, b) } else { Color::Rgb { r, g, b } }
    }

    fn green(&self) -> Color {
        self.rgb([0x4c, 0xae, 0x5a])
    }

    fn red(&self) -> Color {
        self.rgb([0xd9, 0x4f, 0x45])
    }

    /// Accents are mid-tones so they read on dark and light backgrounds alike.
    fn theme(&self, kind: Kind) -> Theme {
        let accent = match kind {
            Kind::White => [0x8c, 0x96, 0xa8],
            Kind::Pink => [0xd4, 0x70, 0x8f],
            Kind::Brown => [0xd0, 0x8f, 0x4a],
            Kind::Blue => [0x4f, 0x8f, 0xd6],
            Kind::Violet => [0x9b, 0x74, 0xd6],
        };
        let track = accent.map(|v| ((v as u16 * 35 + 0x8a * 65) / 100) as u8);
        let tile = accent.map(|v| (v as u16 * 22 / 100) as u8);
        Theme { accent: self.rgb(accent), track: self.rgb(track), tile: self.rgb(tile) }
    }
}

/// Nearest xterm-256 color: the 6x6x6 cube or the gray ramp, whichever is closer.
fn cube(r: u8, g: u8, b: u8) -> Color {
    let level = |v: u8| match v {
        0..48 => 0,
        48..115 => 1,
        _ => (v - 35) / 40,
    };
    let value = |l: u8| if l == 0 { 0 } else { 55 + 40 * l as i32 };
    let dist = |x: [i32; 3]| x.iter().zip([r, g, b]).map(|(a, b)| (a - b as i32).pow(2)).sum::<i32>();
    let (lr, lg, lb) = (level(r), level(g), level(b));
    let gray = ((r as i32 + g as i32 + b as i32) / 3 - 3).clamp(0, 230) / 10;
    let gv = 8 + 10 * gray;
    if dist([gv; 3]) < dist([value(lr), value(lg), value(lb)]) {
        Color::AnsiValue(232 + gray as u8)
    } else {
        Color::AnsiValue(16 + 36 * lr + 6 * lg + lb)
    }
}
const LOGO: [&str; 2] = ["  ▃ ▆▁ ▄ ▇  ▂ ", " ▅█▇██▃█▆██▄█ "];
const KEYS: [(&[&str], &str); 5] = [
    (&["1-5", "↓↑", "jk"], "noise"),
    (&["←→", "hl"], "volume"),
    (&["m"], "mute"),
    (&["q", "esc"], "leave (keeps playing)"),
    (&["s"], "stop"),
];
const KEYS_SHORT: [(&[&str], &str); 5] =
    [(&["1-5"], "noise"), (&["←→"], "vol"), (&["m"], "mute"), (&["q"], "leave"), (&["s"], "stop")];

fn hint_width(keys: &[(&[&str], &str)], sep: usize) -> u16 {
    let text: usize =
        keys.iter().map(|(opts, d)| opts.iter().map(|k| k.chars().count()).sum::<usize>() + opts.len() + d.len()).sum();
    (text + sep * (keys.len() - 1)) as u16
}

fn draw(out: &mut Stdout, p: &Palette, state: Option<&State>, problem: Option<&str>) -> io::Result<()> {
    let (cols, rows) = terminal::size()?;
    let error = problem.or(state.and_then(|s| s.error.as_deref()));
    let full = cols >= 58 && rows >= 15;
    let (w, padx, pady) = if full { (46, 4, 2) } else { (38.min(cols), 0, 0) };
    let (bw, bh) = (w + 2 * padx, if error.is_some() { 8 } else { 6 } + 2 * pady);
    let left = cols.saturating_sub(bw) / 2;
    let top = rows.saturating_sub(bh + 2) / 2;
    let (x, y) = (left + padx, top + pady);
    queue!(out, BeginSynchronizedUpdate, Clear(ClearType::All))?;
    if cols < 34 || rows < 8 {
        queue!(out, MoveTo(0, 0), Print("noiseman: terminal too small"), EndSynchronizedUpdate)?;
        return out.flush();
    }
    let t = p.theme(state.map_or(Kind::Brown, |s| s.kind));
    let logo = if state.is_some_and(|s| s.muted) { t.track } else { t.accent };
    if full {
        let rule = "─".repeat(bw as usize - 2);
        queue!(out, SetForegroundColor(t.track), MoveTo(left, top), Print(format!("╭{rule}╮")))?;
        for r in 1..bh - 1 {
            queue!(out, MoveTo(left, top + r), Print("│"), MoveTo(left + bw - 1, top + r), Print("│"))?;
        }
        queue!(out, MoveTo(left, top + bh - 1), Print(format!("╰{rule}╯")))?;
        queue!(out, SetForegroundColor(logo), SetBackgroundColor(t.tile), MoveTo(x, y), Print(LOGO[0]))?;
        queue!(out, MoveTo(x, y + 1), Print(LOGO[1]), ResetColor)?;
        queue!(out, SetForegroundColor(t.accent), SetAttribute(Attribute::Bold))?;
        queue!(out, MoveTo(x + 17, y), Print("noiseman"), SetAttribute(Attribute::Reset), ResetColor)?;
        queue!(out, SetAttribute(Attribute::Dim), MoveTo(x + 17, y + 1), Print("noise generator"))?;
    } else {
        queue!(out, MoveTo(x, y), SetForegroundColor(t.accent), SetAttribute(Attribute::Bold), Print("noiseman"))?;
    }
    queue!(out, SetAttribute(Attribute::Reset), ResetColor)?;
    if let Some(s) = state {
        if s.muted {
            queue!(out, MoveTo(x + w.saturating_sub(7), y), SetAttribute(Attribute::Dim), Print("◌ Muted"))?;
        } else {
            queue!(out, MoveTo(x + w.saturating_sub(9), y), SetForegroundColor(p.green()), Print("● "))?;
            queue!(out, ResetColor, Print("Playing"))?;
        }
        queue!(out, SetAttribute(Attribute::Reset), MoveTo(x, y + 3))?;
        let labels: u16 = Kind::ALL.iter().map(|k| k.label().len() as u16).sum();
        let digits = if full { 2 * Kind::ALL.len() as u16 } else { 0 };
        let gap = " ".repeat((w.saturating_sub(labels + digits) / 4) as usize);
        for (i, k) in Kind::ALL.iter().enumerate() {
            if i > 0 {
                queue!(out, Print(&gap))?;
            }
            let selected = *k == s.kind;
            if full {
                let digit = if selected { SetForegroundColor(t.accent) } else { SetForegroundColor(t.track) };
                queue!(out, digit, Print(i + 1), Print(" "), ResetColor)?;
            }
            if selected {
                let mark = if p.mono { Attribute::Underlined } else { Attribute::Bold };
                queue!(out, SetForegroundColor(t.accent), SetAttribute(Attribute::Bold), SetAttribute(mark))?;
                queue!(out, Print(k.label()))?;
                queue!(out, SetAttribute(Attribute::Reset), ResetColor)?;
            } else {
                queue!(out, Print(k.label()))?;
            }
        }
        let bar = w.saturating_sub(5) as i32;
        let filled = (s.vol.clamp(0, 100) * bar / 100) as usize;
        let fill = if s.muted { t.track } else { t.accent };
        let (on, off) = if p.mono { ("━", "─") } else { ("▬", "▬") };
        queue!(out, MoveTo(x, y + 5), SetForegroundColor(fill), Print(on.repeat(filled)), SetForegroundColor(t.track))?;
        let rest = off.repeat(bar as usize - filled);
        queue!(out, SetAttribute(Attribute::Dim), Print(rest), SetAttribute(Attribute::Reset))?;
        queue!(out, ResetColor, Print(format!("{:>5}", format!("{}%", s.vol))))?;
    }
    if let Some(e) = error {
        let e: String = format!("✕ {e}").chars().take(w as usize).collect();
        queue!(out, MoveTo(x, y + 7), SetForegroundColor(p.red()), Print(e), ResetColor)?;
    }
    let (keys, sep) = if hint_width(&KEYS, 3) <= cols { (&KEYS, " · ") } else { (&KEYS_SHORT, " ") };
    queue!(out, MoveTo(cols.saturating_sub(hint_width(keys, sep.chars().count())) / 2, top + bh + 1))?;
    for (i, (opts, desc)) in keys.iter().enumerate() {
        if i > 0 {
            queue!(
                out,
                SetForegroundColor(t.track),
                SetAttribute(Attribute::Dim),
                Print(sep),
                SetAttribute(Attribute::Reset)
            )?;
        }
        for (j, key) in opts.iter().enumerate() {
            if j > 0 {
                queue!(
                    out,
                    SetForegroundColor(t.track),
                    SetAttribute(Attribute::Dim),
                    Print("/"),
                    SetAttribute(Attribute::Reset)
                )?;
            }
            queue!(out, SetForegroundColor(t.accent), SetAttribute(Attribute::Bold), Print(key))?;
            queue!(out, SetAttribute(Attribute::Reset))?;
        }
        queue!(out, ResetColor, SetAttribute(Attribute::Dim), Print(" "), Print(desc), SetAttribute(Attribute::Reset))?;
    }
    queue!(out, EndSynchronizedUpdate)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_picks_nearest_xterm_color() {
        assert_eq!(cube(0xd0, 0x8f, 0x4a), Color::AnsiValue(173));
        assert_eq!(cube(0x2e, 0x21, 0x18), Color::AnsiValue(235));
        assert_eq!(cube(0, 0, 0), Color::AnsiValue(16));
        assert_eq!(cube(255, 255, 255), Color::AnsiValue(231));
    }
}
