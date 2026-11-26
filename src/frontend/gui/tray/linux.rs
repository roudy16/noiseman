use std::sync::mpsc::{self, Sender};
use std::thread;

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, OfflineReason, ToolTip};
use winit::event_loop::EventLoopProxy;

use super::{Event, TrayState, description, send_or_log};

pub struct Item {
    proxy: EventLoopProxy<Event>,
    state: TrayState,
}

impl Item {
    fn send(&self, e: Event) {
        send_or_log(&self.proxy, e, "forward tray event");
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "noiseman".into()
    }

    fn title(&self) -> String {
        "noiseman".into()
    }

    fn icon_name(&self) -> String {
        "noiseman".into()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "noiseman".into(), description: description(self.state), ..Default::default() }
    }

    fn activate(&mut self, _: i32, _: i32) {
        self.send(Event::ToggleWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: if self.state.shown { "Hide window" } else { "Show window" }.into(),
                activate: Box::new(|t: &mut Self| t.send(Event::ToggleWindow)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Mute".into(),
                checked: self.state.muted,
                activate: Box::new(|t: &mut Self| t.send(Event::ToggleMute)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| t.send(Event::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_offline(&self, reason: OfflineReason) -> bool {
        eprintln!("noiseman: tray watcher offline ({reason:?}), closing the window will quit");
        self.send(Event::TrayGone);
        false
    }
}

pub struct Tray {
    tx: Sender<TrayState>,
    last: Option<TrayState>,
}

// D-Bus calls have no timeout, so they stay off the winit thread: a stalled watcher must not freeze the UI.
pub fn spawn(proxy: EventLoopProxy<Event>, state: TrayState) {
    thread::spawn(move || {
        let handle = match (Item { proxy: proxy.clone(), state }).spawn() {
            Ok(h) => h,
            Err(e) => {
                eprintln!("noiseman: no system tray, closing the window will quit: {e}");
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        send_or_log(&proxy, Event::TrayReady(Tray { tx, last: None }), "forward tray event");
        for s in rx {
            if handle.update(|i| i.state = s).is_none() {
                eprintln!("noiseman: tray service stopped, closing the window will quit");
                send_or_log(&proxy, Event::TrayGone, "forward tray event");
                return;
            }
        }
    });
}

impl Tray {
    pub fn sync(&mut self, state: TrayState) {
        if self.last != Some(state) {
            self.last = Some(state);
            if let Err(e) = self.tx.send(state) {
                eprintln!("noiseman: cannot update tray, its thread is gone: {e}");
            }
        }
    }
}
