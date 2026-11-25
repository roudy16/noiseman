//! On Windows, embeds `assets/noiseman.ico` (resource 1: Explorer, shortcuts, the taskbar and the window show it) and
//! version info (Task Manager and Properties show the name). Nothing on other systems.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/noiseman.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/noiseman.ico").set("FileDescription", "noiseman").set("ProductName", "noiseman");
        if let Err(e) = res.compile() {
            panic!("cannot embed the Windows icon (needs the Windows SDK's rc.exe, which MSVC installs): {e}");
        }
    }
}
