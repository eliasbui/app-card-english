// The icon of the .exe on Windows; the other platforms take theirs from the bundle or the window.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.compile().expect("Windows resources");
    }
}
