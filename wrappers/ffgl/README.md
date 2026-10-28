# VFX Preview for Resolume (FFGL)

The vfx-core effect chain as an FFGL 2.1 effect for Resolume Avenue and Arena, and
other FFGL 2 hosts.

```bash
wrappers/ffgl/bundle-mac.sh --install
```

This builds a universal `VfxPreview.bundle` and copies it into
`~/Documents/Resolume Avenue/Extra Effects` (and Arena's, if present). Restart Resolume,
then find **VFX Preview** in the Effects browser.

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
instead, and if the GPU fails it draws the input unchanged.
`~/Library/Logs/VfxFfgl.log` records which path ran. `VFX_FFGL_PATH=ram` forces the RAM
path.

Measured on an Apple M5 with blur radius 8 and the vignette on:

| | 720p | 1080p |
|---|---|---|
| GPU-native (IOSurface) | 3.3 ms | 6.4 ms |
| Through RAM | 11.2 ms | 24.0 ms |

## Testing without Resolume

```bash
wrappers/ffgl/minihost/run.sh
```

`minihost/minihost.c` is a small FFGL host with a headless OpenGL 4.1 core context. It
checks the plugin's info and parameter metadata, renders generated textures (including
padded allocations) and checks the pixels, checks the GL state the plugin leaves
behind, and prints frame times.

## Not yet

- Windows: Resolume uses OpenGL there too, and sharing textures with wgpu needs
  vendor-specific interop, so the RAM path would be the starting point.
- A thumbnail for Resolume's effect browser, and beat-synced parameters.
