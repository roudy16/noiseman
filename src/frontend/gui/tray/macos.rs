use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{ClassType, DeclaredClass, declare_class, msg_send_id, mutability, sel};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusBarButton, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{MainThreadMarker, NSData, NSSize, NSString, ns_string};
use winit::event_loop::EventLoopProxy;

use super::{Event, TrayState, description, send_or_log};

const ICON: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/noiseman-menubar.png"));

declare_class!(
    /// The menu items' target: it only forwards their actions to the winit loop.
    struct Target;

    unsafe impl ClassType for Target {
        type Super = objc2_foundation::NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "NoisemanMenuTarget";
    }

    impl DeclaredClass for Target {
        type Ivars = EventLoopProxy<Event>;
    }

    unsafe impl Target {
        #[method(toggleWindow:)]
        fn toggle_window(&self, _: Option<&AnyObject>) {
            self.send(Event::ToggleWindow);
        }

        #[method(toggleMute:)]
        fn toggle_mute(&self, _: Option<&AnyObject>) {
            self.send(Event::ToggleMute);
        }

        #[method(quit:)]
        fn quit(&self, _: Option<&AnyObject>) {
            self.send(Event::Quit);
        }
    }
);

impl Target {
    fn send(&self, e: Event) {
        send_or_log(self.ivars(), e, "forward menu bar event");
    }
}

/// A status item in the menu bar; clicking it opens its menu.
pub struct Tray {
    item: Retained<NSStatusItem>,
    button: Retained<NSStatusBarButton>,
    toggle: Retained<NSMenuItem>,
    mute: Retained<NSMenuItem>,
    // Menu items do not retain their target.
    _target: Retained<Target>,
    last: Option<TrayState>,
}

// SAFETY: Event is Send because other threads send events to the winit loop. A Tray is made on the main thread, which
// runs that loop, sent through it to the same thread, and used and dropped only there.
unsafe impl Send for Tray {}

/// Called from `resumed`: a status item made before the app has finished launching can misbehave.
pub fn spawn(proxy: EventLoopProxy<Event>, state: TrayState) {
    match create(proxy.clone()) {
        Ok(mut t) => {
            t.sync(state);
            send_or_log(&proxy, Event::TrayReady(t), "forward tray event");
        }
        Err(e) => eprintln!("noiseman: no menu bar icon, closing the window will quit: {e}"),
    }
}

fn create(proxy: EventLoopProxy<Event>) -> Result<Tray, String> {
    let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(ICON)).ok_or("cannot decode its icon")?;
    let target: Retained<Target> = unsafe { msg_send_id![super(mtm.alloc().set_ivars(proxy)), init] };
    let menu = NSMenu::new(mtm);
    let add = |title: &NSString, action| unsafe {
        let item = menu.addItemWithTitle_action_keyEquivalent(title, Some(action), ns_string!(""));
        item.setTarget(Some(&target));
        item
    };
    let toggle = add(ns_string!("Hide window"), sel!(toggleWindow:));
    let mute = add(ns_string!("Mute"), sel!(toggleMute:));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(ns_string!("Quit"), sel!(quit:));
    unsafe {
        // Drawn at 2x by tools/icons.py; a template image takes the menu bar's light or dark color.
        let size = image.size();
        image.setSize(NSSize::new(size.width / 2.0, size.height / 2.0));
        image.setTemplate(true);
        image.setAccessibilityDescription(Some(ns_string!("noiseman")));
        let bar = NSStatusBar::systemStatusBar();
        let item = bar.statusItemWithLength(NSVariableStatusItemLength);
        let Some(button) = item.button(mtm) else {
            bar.removeStatusItem(&item);
            return Err("the status bar gave it no button".into());
        };
        button.setImage(Some(&image));
        item.setMenu(Some(&menu));
        Ok(Tray { item, button, toggle, mute, _target: target, last: None })
    }
}

impl Tray {
    pub fn sync(&mut self, state: TrayState) {
        if self.last == Some(state) {
            return;
        }
        let tip = NSString::from_str(&format!("noiseman\n{}", description(state)));
        unsafe {
            self.toggle.setTitle(if state.shown { ns_string!("Hide window") } else { ns_string!("Show window") });
            self.mute.setState(if state.muted { NSControlStateValueOn } else { NSControlStateValueOff });
            self.button.setToolTip(Some(&tip));
        }
        if self.last.is_some_and(|l| l.shown != state.shown) {
            let app = NSApplication::sharedApplication(MainThreadMarker::from(&*self.toggle));
            // An accessory app is not activated by its menu, and keeps the focus after its last window closes.
            if state.shown {
                unsafe { app.unhide(None) };
                #[allow(deprecated)] // NSApp.activate needs macOS 14.
                app.activateIgnoringOtherApps(true);
            } else {
                app.hide(None);
            }
        }
        self.last = Some(state);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe { NSStatusBar::systemStatusBar().removeStatusItem(&self.item) };
    }
}
