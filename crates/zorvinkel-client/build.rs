fn main() {
    println!("cargo:rerun-if-changed=assets/zorvinkel.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_icon();
    }
}

#[cfg(windows)]
fn embed_windows_icon() {
    let mut resources = winresource::WindowsResource::new();
    resources.set_icon("assets/zorvinkel.ico");
    if let Err(error) = resources.compile() {
        println!("cargo:warning=Windows icon was not embedded: {error}");
    }
}

#[cfg(not(windows))]
fn embed_windows_icon() {}
