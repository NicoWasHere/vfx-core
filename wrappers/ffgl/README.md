# The plugin for Resolume (FFGL)

The vfx-core effect chain as an FFGL 2.1 effect for Resolume Avenue and Arena, and
other FFGL 2 hosts.

On macOS:

```bash
wrappers/ffgl/bundle-mac.sh --install
```

This builds a universal `VfxPreview.bundle` (named from `plugin.env`) and copies it into
`~/Documents/Resolume Avenue/Extra Effects` (and Arena's, if present). Restart Resolume,
then find the plugin (**VFX Preview** by default) in the Effects browser.

On Windows, from Git Bash with Rust installed:

```bash
wrappers/ffgl/bundle-win.sh --install
```

This builds `target/vfx-plugin/VfxPreview.dll` and copies it into the same Extra
Effects folders under the user's Documents, when the Resolume folders exist.
Without `--install` it only builds the DLL. The plugin needs a 64-bit FFGL host
with OpenGL 4.1. It passes the mini host on Windows; Resolume there is untested.

## Controls

Linear Light, then **Color** (brightness, contrast, saturation), **Vignette** (centre,
radius, softness, colour, amount) and **Blur** (radius in composition pixels,
direction). Blend and Blur Edges from the other plugins are left out: a second input
would make this an FFGL mixer, and FFGL output is always the frame size.

## How frames move

On macOS the frame stays on the GPU. The host's texture is drawn into an IOSurface,
which the plugin's Metal device renders from and back into a second IOSurface, and that
is drawn into Resolume's framebuffer. The conversions FFGL needs (premultiplied alpha,
bottom-up rows) happen in those two draws.

If the shared surfaces can't be set up, the plugin reads the texture back through RAM
instead, and if the GPU fails it draws the input unchanged. Windows uses this RAM
path. Headless GPU setup uses Vulkan, Metal or DX12 so wgpu leaves the host's
OpenGL context alone.

`~/Library/Logs/VfxPreview-FFGL.log` records which path ran on macOS. On Windows the
log is `%TEMP%\VfxPreview-FFGL.log`. `VFX_FFGL_PATH=ram` forces the RAM path.

Measured on an Apple M5 with blur radius 8 and the vignette on:

| | 720p | 1080p |
|---|---|---|
| GPU-native (IOSurface) | 3.3 ms | 6.4 ms |
| Through RAM | 11.2 ms | 24.0 ms |

On Windows with an AMD Radeon RX 5700 XT 50th Anniversary, the release mini host
measured the same effect settings over 30 frames after a warm-up frame:

| | 720p | 1080p |
|---|---|---|
| Through RAM | 43.5 ms | 88.2 ms |

## Testing without Resolume

```bash
wrappers/ffgl/minihost/run.sh
```

`minihost/minihost.c` is a small FFGL host with a headless OpenGL 4.1 core context. It
checks the plugin's info and parameter metadata, renders generated textures (including
padded allocations) and checks the pixels, checks the GL state the plugin leaves
behind, and prints frame times. On Windows, run it from Git Bash with a C compiler
set in `CC` (default `cc`). It also checks that the host's context and device context
stay current across rendering, including the first GPU setup. The Windows mini host
runs locally; CI runs the unit tests and builds the DLL.

## Not yet

- Resolume on Windows has not been tested. The Windows RAM path passes the mini host.
- GPU-native frames on Windows. Sharing textures with wgpu needs vendor-specific interop.
- A thumbnail for Resolume's effect browser, and beat-synced parameters.
