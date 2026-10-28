use pipl::{OutFlags, OutFlags2, PIPLType, Property, Stage};

/// Plugin description read by After Effects: the PiPL resource. Name, match name and
/// category must agree with `PluginDataEntryFunction3` in `src/ae.rs`. The match name is not
/// segcam's C++ effect's, so both can be installed side by side.
fn properties() -> Vec<Property> {
    vec![
        Property::Kind(PIPLType::AEEffect),
        Property::Name("segcam Motion (Rust)"),
        Property::Category("LO.FI.SCI.FI"),
        Property::CodeWin64X86("EffectMain"),
        Property::CodeMacIntel64("EffectMain"),
        Property::CodeMacARM64("EffectMain"),
        Property::AE_PiPL_Version { major: 2, minor: 0 },
        // After Effects disables Multi-Frame Rendering below 13.25.
        Property::AE_Effect_Spec_Version {
            major: 13,
            minor: 29,
        },
        Property::AE_Effect_Version {
            version: 1,
            subversion: 0,
            bugversion: 0,
            stage: Stage::Develop,
            build: 1,
        },
        Property::AE_Effect_Info_Flags(0),
        // Wide time input: every render reads the frames before the current one. The
        // after-effects crate keeps sequence data as a Rust handle, so After Effects must
        // flatten it before saving.
        Property::AE_Effect_Global_OutFlags(
            OutFlags::DeepColorAware
                | OutFlags::WideTimeInput
                | OutFlags::SequenceDataNeedsFlattening,
        ),
        // Each render is self-contained (it rebuilds its background from checked-out
        // frames), so Multi-Frame Rendering is safe.
        Property::AE_Effect_Global_OutFlags_2(
            OutFlags2::FloatColorAware
                | OutFlags2::SupportsSmartRender
                | OutFlags2::SupportsThreadedRendering
                | OutFlags2::SupportsGetFlattenedSequenceData,
        ),
        Property::AE_Effect_Match_Name("LOFISCIFI segcam Motion Rust"),
    ]
}

fn main() {
    // Sets the PIPL_* env vars that the after-effects crate reads at compile time.
    pipl::plugin_build(properties());

    // Also writes the PiPL as a classic resource file, which the macOS bundle needs.
    let bytes = pipl::build_pipl(properties()).expect("build PiPL");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("SegcamMotion.rsrc");
    pipl::produce_resource(&bytes, out.to_str());
    println!("cargo:rustc-env=PIPL_AE_RESERVED=0");
    println!("cargo:rustc-env=PIPL_SUPPORT_URL=");
    // After Effects only loads Mach-O bundles (MH_BUNDLE), not dylibs. Cargo's cdylib links as a dylib.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-bundle");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
