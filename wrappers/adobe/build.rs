use pipl::{OutFlags, OutFlags2, PIPLType, Property, Stage};

include!("../common/plugin_env.rs");

/// The names After Effects shows and matches the effect by, from plugin.env.
struct Names {
    name: &'static str,
    category: &'static str,
    match_name: &'static str,
}

/// Plugin description read by After Effects: the PiPL resource.
fn properties(names: &Names) -> Vec<Property> {
    vec![
        Property::Kind(PIPLType::AEEffect),
        Property::Name(names.name),
        Property::Category(names.category),
        Property::CodeWin64X86("EffectMain"),
        Property::CodeMacIntel64("EffectMain"),
        Property::CodeMacARM64("EffectMain"),
        Property::AE_PiPL_Version { major: 2, minor: 0 },
        // The SDK the after-effects-sys bindings come from (PF_AE_PLUG_IN_VERSION and
        // _SUBVERS). After Effects disables Multi-Frame Rendering below 13.25.
        Property::AE_Effect_Spec_Version {
            major: 13,
            minor: 29,
        },
        Property::AE_Effect_Version {
            version: 1,
            subversion: 1,
            bugversion: 0,
            stage: Stage::Develop,
            build: 0,
        },
        Property::AE_Effect_Info_Flags(0),
        // The after-effects crate stores sequence data as a Rust handle and restores it
        // from flattened bytes, so After Effects must flatten it before saving.
        Property::AE_Effect_Global_OutFlags(
            OutFlags::DeepColorAware | OutFlags::SequenceDataNeedsFlattening,
        ),
        // 32 bpc float requires SmartFX, so those two flags go together. Threaded
        // rendering (Multi-Frame Rendering) is safe because each render takes its own
        // chain from the pool and the GPU device and queue are thread-safe. With
        // flattened sequence data it also needs GET_FLATTENED_SEQUENCE_DATA, which the
        // crate handles.
        Property::AE_Effect_Global_OutFlags_2(
            OutFlags2::FloatColorAware
                | OutFlags2::SupportsSmartRender
                | OutFlags2::SupportsThreadedRendering
                | OutFlags2::SupportsGetFlattenedSequenceData
                | gpu_flag(),
        ),
        Property::AE_Effect_Match_Name(names.match_name),
    ]
}

/// GPU-native rendering exists only for Metal so far, so only macOS builds advertise it.
fn gpu_flag() -> OutFlags2 {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        OutFlags2::SupportsGpuRenderF32
    } else {
        OutFlags2::empty()
    }
}

fn main() {
    let values = plugin_env();
    // The PiPL needs 'static strings; a build script runs once, so leaking is fine.
    let value = |key| -> &'static str { plugin_value(&values, key).to_owned().leak() };
    let names = Names {
        name: value("PLUGIN_NAME"),
        category: value("PLUGIN_CATEGORY"),
        match_name: value("AE_MATCH_NAME"),
    };

    // Sets the PIPL_* env vars that the after-effects crate reads at compile time.
    pipl::plugin_build(properties(&names));

    // Also writes the PiPL as a classic resource file, which the macOS bundle needs.
    let bytes = pipl::build_pipl(properties(&names)).expect("build PiPL");
    let rsrc = format!("{}.rsrc", value("PLUGIN_FILE"));
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(rsrc);
    pipl::produce_resource(&bytes, out.to_str());
    // Fields the after-effects macro reads at compile time. Neither has a value to set yet.
    println!("cargo:rustc-env=PIPL_AE_RESERVED=0");
    println!("cargo:rustc-env=PIPL_SUPPORT_URL=");
    // After Effects only loads Mach-O bundles (MH_BUNDLE), not dylibs. Cargo's cdylib links as a dylib.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-bundle");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
