//! Gives massif.exe its icon on Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Massif");
        res.set("FileDescription", "Massif, a map editor for Minecraft worlds");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no icon for massif.exe: {e}");
        }
    }
}
