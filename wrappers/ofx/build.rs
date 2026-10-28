include!("../common/plugin_env.rs");

fn main() {
    // The plugin's identifier, name and grouping, as VFX_* compile-time variables.
    plugin_env();
    // OpenFX hosts load the binary with dlopen. Link it as a Mach-O bundle, as the
    // OpenFX samples do, rather than a dylib.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-bundle");
    }
}
