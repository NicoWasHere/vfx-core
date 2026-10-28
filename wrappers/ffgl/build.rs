fn main() {
    // Resolume loads FFGL plugins as Mach-O bundles, like the FFGL SDK builds them.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-bundle");
    }
}
