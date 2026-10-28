// Reads the plugin's identity from plugin.env at the repository root. Included by each
// wrapper's build.rs, which passes the values on to the crate as `cargo:rustc-env`
// variables (VFX_PLUGIN_NAME and so on), read in the code with `env!`.

/// Every key in plugin.env, checked and returned in this order.
const PLUGIN_KEYS: [&str; 7] = [
    "PLUGIN_NAME",
    "PLUGIN_CATEGORY",
    "PLUGIN_FILE",
    "PLUGIN_ID",
    "AE_MATCH_NAME",
    "FFGL_ID",
    "WINDOWS_INSTALLER_ID",
];

/// Parses plugin.env, sets a `VFX_<KEY>` compile-time variable for each key, and returns
/// the values as (key, value) pairs.
fn plugin_env() -> Vec<(&'static str, String)> {
    let path = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../plugin.env");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let mut found = std::collections::HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parsed = line.split_once('=').and_then(|(key, rest)| {
            let value = rest.strip_prefix('"')?.split('"').next()?;
            Some((key.trim().to_owned(), value.to_owned()))
        });
        let Some((key, value)) = parsed else {
            panic!("plugin.env: expected KEY=\"value\", got {line:?}");
        };
        found.insert(key, value);
    }
    PLUGIN_KEYS
        .iter()
        .map(|&key| {
            let value = found
                .remove(key)
                .unwrap_or_else(|| panic!("plugin.env is missing {key}"));
            assert!(!value.is_empty(), "plugin.env: {key} is empty");
            println!("cargo:rustc-env=VFX_{key}={value}");
            (key, value)
        })
        .collect()
}

/// The value of one key from `plugin_env()`.
#[allow(dead_code)] // Not every build script needs single values.
fn plugin_value<'a>(values: &'a [(&'static str, String)], key: &str) -> &'a str {
    &values.iter().find(|(k, _)| *k == key).expect("known key").1
}
