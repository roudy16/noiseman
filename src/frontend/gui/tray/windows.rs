use std::cell::Cell;
use std::io;
use std::ptr::{null, null_mut, without_provenance};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
    NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
    FindWindowW, GWLP_USERDATA, GetWindowLongPtrW, HICON, IMAGE_ICON, LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    PostMessageW, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow,
    SetWindowLongPtrW, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
    WM_NULL, WNDCLASSW,
};
use windows_sys::w;
use winit::event_loop::EventLoopProxy;

use super::{Event, TrayState, description, send_or_log};

const WM_TRAY: u32 = WM_APP + 1;
const TOGGLE: usize = 1;
const MUTE: usize = 2;
const QUIT: usize = 3;

/// What the window procedure needs, reached through the window's `GWLP_USERDATA`.
struct Item {
    proxy: EventLoopProxy<Event>,
    state: Cell<TrayState>,
    icon: Cell<HICON>,
    taskbar_created: u32,
}

impl Drop for Item {
    fn drop(&mut self) {
        if !self.icon.get().is_null() {
            unsafe { DestroyIcon(self.icon.get()) };
        }
    }
}

/// A hidden window, on the winit thread, whose messages winit's pump dispatches, and the icon it owns.
pub struct Tray {
    hwnd: HWND,
    item: *const Item,
}

// SAFETY: Event is Send because other threads send events to the winit loop. A Tray is made on the winit thread, sent
// through that loop to the same thread, and used and dropped only there.
unsafe impl Send for Tray {}

pub fn spawn(proxy: EventLoopProxy<Event>, state: TrayState) {
    match create(proxy.clone(), state) {
        Ok(t) => send_or_log(&proxy, Event::TrayReady(t), "forward tray event"),
        Err(e) => eprintln!("noiseman: no notification area icon, closing the window will quit: {e}"),
    }
}

fn create(proxy: EventLoopProxy<Event>, state: TrayState) -> Result<Tray, String> {
    let class = w!("noiseman-tray");
    unsafe {
        let module = GetModuleHandleW(null());
        let wc =
            WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: module, lpszClassName: class, ..Default::default() };
        if RegisterClassW(&wc) == 0 {
            return Err(format!("cannot register its window class: {}", io::Error::last_os_error()));
        }
        // Top-level but never shown: a message-only window would miss the TaskbarCreated broadcast.
        let hwnd = CreateWindowExW(0, class, null(), 0, 0, 0, 0, 0, null_mut(), null_mut(), module, null());
        if hwnd.is_null() {
            return Err(format!("cannot create its window: {}", io::Error::last_os_error()));
        }
        let item = Box::into_raw(Box::new(Item {
            proxy,
            state: Cell::new(state),
            icon: Cell::new(null_mut()),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        }));
        let tray = Tray { hwnd, item };
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, item as isize);
        add(hwnd, &*item)?;
        Ok(tray)
    }
}

fn notify(msg: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> bool {
    unsafe { Shell_NotifyIconW(msg, data) != 0 }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

fn with_tip(hwnd: HWND, state: TrayState) -> NOTIFYICONDATAW {
    let mut d = data(hwnd);
    d.uFlags = NIF_TIP | NIF_SHOWTIP;
    let tip = format!("noiseman\n{}", description(state));
    let len = d.szTip.len() - 1;
    for (to, from) in d.szTip[..len].iter_mut().zip(tip.encode_utf16()) {
        *to = from;
    }
    d
}

/// Resource 1, the icon build.rs embeds, at the small icon size for the taskbar's DPI now. GetSystemMetrics would give
/// the size for the DPI at sign-in, and LR_SHARED the first size loaded whatever the size asked for.
fn load_icon() -> HICON {
    unsafe {
        let dpi = match GetDpiForWindow(FindWindowW(w!("Shell_TrayWnd"), null())) {
            0 => GetDpiForSystem(),
            d => d,
        };
        let (w, h) = (GetSystemMetricsForDpi(SM_CXSMICON, dpi), GetSystemMetricsForDpi(SM_CYSMICON, dpi));
        LoadImageW(GetModuleHandleW(null()), without_provenance(1), IMAGE_ICON, w, h, 0)
    }
}

/// Version 4 sends NIN_SELECT for a click and WM_CONTEXTMENU for a right click or the menu key, with the anchor point.
fn add(hwnd: HWND, item: &Item) -> Result<(), String> {
    let icon = load_icon();
    if icon.is_null() {
        return Err(format!("cannot load its icon: {}", io::Error::last_os_error()));
    }
    let old = item.icon.replace(icon);
    if !old.is_null() {
        unsafe { DestroyIcon(old) };
    }
    let mut d = with_tip(hwnd, item.state.get());
    d.uFlags |= NIF_MESSAGE | NIF_ICON;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = icon;
    d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    if notify(NIM_ADD, &d) && notify(NIM_SETVERSION, &d) { Ok(()) } else { Err("the shell refused it".into()) }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(item) = (unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Item).as_ref() }) else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    if msg == WM_TRAY {
        match lparam as u32 & 0xffff {
            NIN_SELECT => send_or_log(&item.proxy, Event::ToggleWindow, "forward tray event"),
            WM_CONTEXTMENU => {
                let (x, y) = (wparam as i16 as i32, (wparam >> 16) as i16 as i32);
                // The menu runs a modal loop, during which the Tray may be dropped, so it gets copies.
                menu(hwnd, item.proxy.clone(), item.state.get(), x, y);
            }
            _ => {}
        }
    } else if msg == item.taskbar_created {
        // Explorer restarted, or the taskbar changed DPI and still shows the icon, so remove it before adding it at the
        // new size.
        notify(NIM_DELETE, &data(hwnd));
        if let Err(e) = add(hwnd, item) {
            eprintln!("noiseman: cannot add the notification area icon again, closing the window will quit: {e}");
            send_or_log(&item.proxy, Event::TrayGone, "forward tray event");
        }
    } else if msg == WM_CLOSE {
        // A graceful kill (taskkill without /F) closes every top-level window; DefWindowProcW would destroy this one
        // and leave an icon that ignores clicks while the hidden window keeps playing.
        send_or_log(&item.proxy, Event::Quit, "forward tray event");
    } else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    0
}

fn menu(hwnd: HWND, proxy: EventLoopProxy<Event>, state: TrayState, x: i32, y: i32) {
    let toggle = if state.shown { w!("Hide window") } else { w!("Show window") };
    let cmd = unsafe {
        let m = CreatePopupMenu();
        if m.is_null() {
            eprintln!("noiseman: cannot open the tray menu: {}", io::Error::last_os_error());
            return;
        }
        AppendMenuW(m, MF_STRING, TOGGLE, toggle);
        AppendMenuW(m, MF_STRING | if state.muted { MF_CHECKED } else { 0 }, MUTE, w!("Mute"));
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        AppendMenuW(m, MF_STRING, QUIT, w!("Quit"));
        // Without the foreground the menu would not close on a click elsewhere; WM_NULL is the documented follow-up.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, x, y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        cmd as usize
    };
    let event = match cmd {
        TOGGLE => Event::ToggleWindow,
        MUTE => Event::ToggleMute,
        QUIT => Event::Quit,
        _ => return,
    };
    send_or_log(&proxy, event, "forward tray event");
}

impl Tray {
    pub fn sync(&mut self, state: TrayState) {
        let item = unsafe { &*self.item };
        if item.state.replace(state) != state && !notify(NIM_MODIFY, &with_tip(self.hwnd, state)) {
            eprintln!("noiseman: cannot update the notification area icon");
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        // Fails harmlessly when the icon is already gone: never added, or lost with Explorer.
        notify(NIM_DELETE, &data(self.hwnd));
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            if DestroyWindow(self.hwnd) == 0 {
                eprintln!("noiseman: cannot destroy the tray window: {}", io::Error::last_os_error());
            }
            drop(Box::from_raw(self.item.cast_mut()));
        }
    }
}
