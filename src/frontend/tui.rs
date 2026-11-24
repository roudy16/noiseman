use std::io::{self, Stdout, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

use super::client::{Client, Sender};
use crate::protocol::{Command, Kind, State};

const BAR: i32 = 20;

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
            Ui::Input(Event::Key(key)) => match action(key) {
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
        draw(&mut screen.0, state.as_ref(), problem.as_deref()).map_err(|e| format!("cannot draw: {e}"))?;
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

fn action(key: KeyEvent) -> Action {
    if key.kind != KeyEventKind::Press {
        return Action::None;
    }
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Leave,
        _ if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => Action::None,
        KeyCode::Char('q') | KeyCode::Esc => Action::Leave,
        KeyCode::Char('s') => Action::Stop,
        KeyCode::Char('m' | ' ') => Action::Send(Command::ToggleMute),
        KeyCode::Left | KeyCode::Char('-' | 'h') => Action::Send(Command::VolBy(-5)),
        KeyCode::Right | KeyCode::Char('+' | '=' | 'l') => Action::Send(Command::VolBy(5)),
        KeyCode::Char(c @ '1'..='5') => Action::Send(Command::Kind(Kind::ALL[c as usize - '1' as usize])),
        _ => Action::None,
    }
}

fn draw(out: &mut Stdout, state: Option<&State>, problem: Option<&str>) -> io::Result<()> {
    queue!(out, Clear(ClearType::All), MoveTo(1, 1), SetAttribute(Attribute::Bold), Print("noiseman"))?;
    queue!(out, SetAttribute(Attribute::Reset))?;
    if let Some(s) = state {
        queue!(out, MoveTo(1, 3))?;
        for (i, k) in Kind::ALL.iter().enumerate() {
            let attr = if *k == s.kind { Attribute::Reverse } else { Attribute::NoReverse };
            queue!(out, SetAttribute(attr), Print(format!(" {} {} ", i + 1, k.label())), SetAttribute(Attribute::Reset))?;
            queue!(out, Print(" "))?;
        }
        let filled = (s.vol * BAR / 100) as usize;
        let bar = format!("[{}{}] {:>3}%", "#".repeat(filled), "-".repeat(BAR as usize - filled), s.vol);
        queue!(out, MoveTo(1, 5), Print("Volume "), Print(bar))?;
        queue!(out, MoveTo(1, 7), Print(if s.muted { "Muted" } else { "Playing" }))?;
    }
    if let Some(e) = problem.or(state.and_then(|s| s.error.as_deref())) {
        queue!(out, MoveTo(1, 9), SetForegroundColor(Color::Red), Print(e), ResetColor)?;
    }
    queue!(out, MoveTo(1, 11), SetAttribute(Attribute::Dim))?;
    queue!(out, Print("1-5 noise  \u{2190}/\u{2192} volume  m mute  q leave (keeps playing)  s stop"))?;
    queue!(out, SetAttribute(Attribute::Reset))?;
    out.flush()
}
