fn main() {
    println!("cargo:rerun-if-changed=assets/pbrt-ui.ico");
    // The exe icon Explorer and the taskbar show; a native build, so host == target.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/pbrt-ui.ico")
        .compile()
        .expect("embed the Windows icon resource");
}
