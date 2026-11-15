//! Keeps the COM entry points of the Windows library out of its import
//! library. Explorer finds `DllGetClassObject` and `DllCanUnloadNow` by name
//! (`GetProcAddress`), so nothing links against them, and the MSVC linker
//! warns (LNK4104) when they are not exported PRIVATE. rustc exports the
//! `#[no_mangle]` functions through `/EXPORT` directives in an object of its
//! own (`symbols.o`); an `/EXPORT` on the linker's command line takes their
//! place without a warning, the functions still in the DLL's export table.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        for name in ["DllGetClassObject", "DllCanUnloadNow"] {
            println!("cargo:rustc-cdylib-link-arg=/EXPORT:{name},PRIVATE");
        }
    }
}
