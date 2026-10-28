# vfx-core

GPU video effects written once in Rust on [wgpu](https://wgpu.rs), shipped as plugins
for several hosts:

| Target | Crate | Hosts | Status |
|---|---|---|---|
| Core library | `vfx-core` (repo root) | any Rust app | tested headless on macOS, Windows, Linux |
| After Effects | `wrappers/adobe` | After Effects 2026 (Premiere untested) | tested in AE; SmartFX, 8/16/32 bpc, Multi-Frame Rendering, GPU-native on Metal |
| OpenFX | `wrappers/ofx` | DaVinci Resolve, Nuke, Natron… | passes a mini host on macOS and Windows; loads in Resolve 21 on macOS (the Mac App Store build hides third-party OpenFX); renders in Natron 2.5 on Windows |
| FFGL | `wrappers/ffgl` | Resolume Avenue / Arena | tested in Avenue 7.17 on macOS; zero-copy via IOSurface; passes the mini host on Windows through RAM |
| Preview app | `examples/preview` | desktop and browser (WebGPU) | egui control panel; the web build deploys to [GitHub Pages](https://jamecoyne.github.io/vfx-core/) |

Every target renders the same effect chain through the same code, so a frame looks the
same in each host.

Licensed MIT OR Apache-2.0.

## The effects

`src/effects/`: colour adjust (brightness, contrast, saturation), Gaussian blur
(separable, per-axis radius, horizontal / vertical / both), vignette (centre, radius,
softness, colour, amount), blend with a second layer (normal, add, multiply, screen),
feedback trail, and pixel conversions (premultiply, unpremultiply, sRGB ↔ linear).

The plugins build this chain from those effects (`wrappers/common/src/pipeline.rs`):

```
[sRGB → linear] → colour adjust → vignette → blend → [premultiply → blur → unpremultiply] → [linear → sRGB]
```

Stages that are switched off aren't in the chain. Blur runs on premultiplied alpha so
transparent pixels don't bleed colour into soft edges; everything else works on straight
alpha. Frames are RGBA32F, so 32 bpc values above 1.0 survive.

## Layout

```
src/                    vfx-core: Effect trait, typed params, Frame, EffectChain, WGSL passes
  effects/              the effects above
tests/                  headless GPU tests for the core
wrappers/
  common/               shared by every plugin: Settings, the pooled pipeline, render-path log
  adobe/                After Effects SmartFX plugin (after-effects crate), Metal interop
    ae-tests/           harness that renders inside After Effects and checks the frames
    examples/ae_reference.rs   the plugin's pipeline without After Effects, for that harness
  ofx/                  OpenFX plugin, hand-written bindings to the OpenFX 1.4 C API
    minihost/           a small OpenFX host in C for testing without an application
  ffgl/                 FFGL 2.1 plugin for Resolume, OpenGL + IOSurface interop
    minihost/           a small FFGL host in C with a headless OpenGL context
examples/preview/       standalone app: egui panel, native and web
  web/                  index.html and build.sh for the WebGPU build
.github/workflows/      ci.yml (tests, plugin builds), pages.yml (web preview)
```

## Building and running

Needs a current stable Rust (egui needs 1.95+; CI uses latest stable). Plugin bundles
for macOS are universal, so add both targets once:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin wasm32-unknown-unknown
```

| What | Command | Result |
|---|---|---|
| Core tests | `cargo test -p vfx-core` | headless GPU tests |
| All wrapper tests | `cargo test -p vfx-wrapper-common -p vfx-adobe -p vfx-ofx -p vfx-ffgl` | unit tests |
| Preview (desktop) | `cargo run -p vfx-preview --release` | window with control panel; drop a PNG/JPEG on it |
| Preview (web) | `examples/preview/web/build.sh --serve` | http://localhost:8123 (needs `wasm-bindgen-cli` at the lockfile's version) |
| After Effects | `wrappers/adobe/package-mac.sh` | `target/vfx-plugin/VfxAdobe.pkg` → installs to Adobe's MediaCore folder |
| OpenFX | `wrappers/ofx/package-mac.sh` | `target/vfx-plugin/VfxPreview-ofx.pkg` → installs to `/Library/OFX/Plugins` |
| OpenFX (Windows) | `wrappers/ofx/bundle-win.sh --install` | `VfxPreview.ofx.bundle` → `C:\Program Files\Common Files\OFX\Plugins` (run from Git Bash as administrator) |
| Resolume | `wrappers/ffgl/bundle-mac.sh --install` | copies `VfxPreview.bundle` into `~/Documents/Resolume Avenue/Extra Effects` |
| Resolume (Windows) | `wrappers/ffgl/bundle-win.sh --install` | copies `VfxPreview.dll` into the existing Resolume Documents folders (run from Git Bash) |
| Windows installer | `wrappers/package-win.sh` | `target/vfx-plugin/VfxPreview-<version>-Windows-Setup.exe` and a `.zip` of the same files (needs [Inno Setup](https://jrsoftware.org/isinfo.php)) |

The `.pkg` scripts sign and notarize only when `SIGN_IDENTITY`, `NOTARY_PROFILE` and
`INSTALLER_IDENTITY` are set; without them the packages are unsigned (fine locally).
Each plugin writes one-line notes about which render path ran to
`~/Library/Logs/VfxAdobe.log`, `VfxOfx.log` or `VfxFfgl.log` (on Windows, the same names
in `%TEMP%`).

## Releases

`.github/workflows/release.yml` builds every plugin on GitHub's macOS and Windows
runners. Push a version tag and it makes a draft release with the files attached;
check it on GitHub, then publish it:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

Running the workflow by hand (Actions → release → Run workflow) builds the same files
as the run's artifacts, without a release.

| File | What |
|---|---|
| `VfxPreview-<version>-macOS-AfterEffects.pkg` | After Effects and Premiere Pro installer |
| `VfxPreview-<version>-macOS-OpenFX.pkg` | OpenFX installer (Resolve, Nuke, Natron) |
| `VfxPreview-<version>-macOS-Resolume.zip` | the FFGL bundle, for Extra Effects |
| `VfxPreview-<version>-Windows-Setup.exe` | Windows installer: OpenFX and Resolume, After Effects optional |
| `VfxPreview-<version>-Windows.zip` | the same Windows files, to copy by hand |

macOS signing and notarization run when these repository secrets are set (Settings →
Secrets and variables → Actions); without them the packages are unsigned:

- `MACOS_CERTIFICATES_P12`: a base64 `.p12` holding the Developer ID Application and
  Developer ID Installer certificates, and `MACOS_CERTIFICATES_PASSWORD`, its password.
- `APPLE_ID`, `APPLE_TEAM_ID` and `APPLE_APP_PASSWORD` (an app-specific password), for
  notarization.

The Windows installer is unsigned, so Windows SmartScreen asks before running it.

## Testing

Unit tests cover the core, the pipeline and each wrapper's pixel conversions. Beyond
those, each host has its own harness:

- **After Effects**: `python3 wrappers/adobe/ae-tests/run.py` builds fixture projects in
  the running After Effects (8, 16, 32 bpc; CPU and Metal), renders them with `aerender`
  (MFR on and off), and compares against the pipeline run directly. 157 checks, under a
  minute. See its README. Needs After Effects, so it isn't in CI.
- **OpenFX**: `wrappers/ofx/minihost/run.sh` loads the plugin into a C host built on the
  real OpenFX headers and checks pixels, depths, row order, premultiplication, render
  scale, regions of definition and interest. Runs in CI on macOS and Windows. On Windows,
  run it from Git Bash with `CC` and `OFX_INCLUDE` set (CI uses clang and the headers
  from the OpenFX repository).
- **FFGL**: `wrappers/ffgl/minihost/run.sh` does the same for FFGL with a headless OpenGL
  context: metadata, pixels, padded textures, GL state left behind, and frame times.
  `FFGL_MINIHOST_SOFTWARE=1` uses Apple's software renderer, as on macOS CI runners.
  Runs in CI on macOS. On Windows, run from Git Bash with `CC` set; the GPU host runs
  locally, while CI runs unit tests and builds the DLL.

## Design notes

- **One device per plugin, chains pooled.** Building pipelines is the expensive part of
  a render, so prepared chains are kept per (device, size, stages) and leased per render.
  Concurrent renders (After Effects MFR, OpenFX hosts) each lease their own.
- **No OpenGL backend for plugins.** Headless devices use Vulkan, Metal or DX12. wgpu's
  OpenGL backend releases whatever context is current when it starts, which on Windows
  is the FFGL host's. The preview creates its own instance.
- **GPU-native paths where hosts allow it.** After Effects with Metal acceleration hands
  over `MTLBuffer`s: the plugin opens wgpu on After Effects' own device and wraps them.
  Resolume's GL textures are drawn into IOSurfaces that wgpu's Metal device shares. Both
  avoid any CPU copy; everything else (and any setup failure) goes through RAM, and a GPU
  failure passes the frame through unchanged rather than failing the host's render.
- **Hand-written host bindings.** OpenFX and FFGL are small C ABIs, transcribed from
  their headers (BSD-licensed; FFGL's notice is kept in `wrappers/ffgl/src/ffi.rs`). The
  `ffgl-core` crate was avoided because it's GPL-3.0.
- **Host quirks found by testing**, now handled and covered by tests or the harnesses:
  After Effects needs PiPL spec version 13.25+ for MFR and sequence-data flattening flags
  for saved projects; it trims an effect's input to mask bounds; Resolve 21 rejects the
  OpenFX plugin-description property; FFGL textures are premultiplied and bottom-up.

## Known gaps

- Windows:
  - After Effects: the `.aex` builds in CI but hasn't been tested in After Effects.
  - OpenFX: passes the mini host and renders in Natron 2.5, but hasn't been tried in
    Resolve on Windows yet.
  - FFGL: passes the mini host through RAM, but has not been tried in Resolume on
    Windows. GPU-native texture sharing is not done.
- Premiere Pro is untested. Signing and notarization have never been run.
- OpenFX GPU rendering (Resolve's Metal extension) and an FFGL thumbnail aren't done.
- Hosts with no plugin yet: TouchDesigner (a Custom TOP, or the shaders as a GLSL
  `.tox`), and EffectCraft, whose only plugin API is CPU WebAssembly.

## Notes for agents working on this repo

- Read the README in each wrapper (`ae-tests/`, `ffgl/`) before changing that wrapper,
  and run its harness afterwards. The harnesses test the *installed* plugin, so rebuild
  and reinstall first (installing to system folders needs the user's password).
- Effect logic belongs in `src/effects`; anything two plugins share belongs in
  `wrappers/common`. Wrappers should only translate parameters and move pixels.
- Keep parameter names, defaults and ranges identical across the After Effects
  (`adobe/src/params.rs`), OpenFX (`ofx/src/params.rs`), FFGL (`ffgl/src/params.rs`) and
  preview (`examples/preview/src/controls.rs`) panels.
- `cargo fmt --all -- --check` and `cargo clippy -p vfx-core --all-targets -- -D warnings`
  must pass; CI runs latest stable, whose clippy can be stricter than an older local one.
- wgpu is pinned to `=30.0.1`; the Metal interop uses `wgpu-hal` types and must be checked
  when it moves. The web build needs wgpu's `fragile-send-sync-non-atomic-wasm` feature
  (set in `examples/preview/Cargo.toml`) because core effects are `Send`.
