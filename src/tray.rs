use std::sync::mpsc::{self, Sender};
use std::thread;

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, OfflineReason, ToolTip};
use winit::event_loop::EventLoopProxy;

use crate::{Event, TrayState};

pub struct Item {
    proxy: EventLoopProxy<Event>,
    state: TrayState,
}

fn send(proxy: &EventLoopProxy<Event>, e: Event) {
    if let Err(err) = proxy.send_event(e) {
        eprintln!("noiseman: cannot forward tray event, event loop gone: {err}");
    }
}

impl Item {
    fn send(&self, e: Event) {
        send(&self.proxy, e);
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
        let (_, muted, kind) = self.state;
        let description = format!("{} noise{}", kind.label(), if muted { ", muted" } else { "" });
        ToolTip { title: "noiseman".into(), description, ..Default::default() }
    }

    fn activate(&mut self, _: i32, _: i32) {
        self.send(Event::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let (shown, muted, _) = self.state;
        vec![
            StandardItem {
                label: if shown { "Hide window" } else { "Show window" }.into(),
                activate: Box::new(|t: &mut Self| t.send(Event::Toggle)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Mute".into(),
                checked: muted,
                activate: Box::new(|t: &mut Self| t.send(Event::Mute)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut Self| t.send(Event::Quit)), ..Default::default() }
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
            Err(e) => return eprintln!("noiseman: no system tray, closing the window will quit: {e}"),
        };
        let (tx, rx) = mpsc::channel();
        send(&proxy, Event::TrayReady(Tray { tx, last: None }));
        for s in rx {
            if handle.update(|i| i.state = s).is_none() {
                eprintln!("noiseman: tray service stopped, closing the window will quit");
                return send(&proxy, Event::TrayGone);
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
