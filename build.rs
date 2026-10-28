// The icon of the .exe on Windows; the other platforms take theirs from the bundle or the window. The target is
// read from the environment, since cfg(windows) in a build script is the machine that builds, not the one it is for.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile().expect("Windows resources");
    }
}
