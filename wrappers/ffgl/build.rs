include!("../common/plugin_env.rs");

fn main() {
    // The plugin's name and FFGL code, as VFX_* compile-time variables. FFGL keeps both
    // in fixed fields, so check they fit here rather than at load time.
    let values = plugin_env();
    let code = plugin_value(&values, "FFGL_ID");
    assert!(
        code.len() == 4 && code.is_ascii(),
        "plugin.env: FFGL_ID must be exactly 4 ASCII characters, not {code:?}"
    );
    let name = plugin_value(&values, "PLUGIN_NAME");
    assert!(
        name.len() <= 16,
        "plugin.env: FFGL shows at most 16 bytes of PLUGIN_NAME, and {name:?} has {}",
        name.len()
    );
    // Resolume loads FFGL plugins as Mach-O bundles, like the FFGL SDK builds them.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-bundle");
    }
}
