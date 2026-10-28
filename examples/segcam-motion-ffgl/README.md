# segcam Motion for Resolume

segcam's CPU motion effect as an FFGL 2.1 plugin. The host name is **segcam Motion**,
the four-byte ID is `SGCM`, and its file is `SegcamMotionRust.dll` on Windows or
`SegcamMotionRust.bundle` on macOS. It has its own entry point; it shares only the
FFGL bindings and GL helpers with VFX Preview.

The host supplies one texture at a time. The plugin keeps the last background frames
(and the trail in Swarm mode), reusing their ARGB byte buffers. GL crops padded
textures, flips the rows and unpremultiplies the input before the CPU core sees it;
the output draw reverses those conversions. Resizing or backwards host time clears
the history. Increasing Background or Trail frames fills the extra history as new
frames arrive; the host cannot supply earlier frames on demand.

## Checking it

```bash
cargo test -p segcam-motion --no-default-features
cargo test -p segcam-motion-ffgl -p vfx-ffgl-common
examples/segcam-motion-ffgl/minihost/run.sh
```

The shared FFGL mini host checks the metadata, padded textures, GL state and current
Windows context. It times all four modes at 720p and 1080p after filling the history.
`compare.py` then sends twenty 48-frame moving-object sequences through the plugin
and the standalone CLI: exact and padded allocations, partial and zero alpha,
non-default settings, backwards time and resize. The outputs must match byte for
byte after converting the CLI's straight ARGB to FFGL's premultiplied ARGB.

Run from Git Bash on Windows with `CC` set to a C compiler. Python 3 is needed for
the comparison; it uses only the standard library. `segcam` skips the frame-time
benchmark; `segcam-timing` runs only that benchmark. `--work-dir PATH` after the case
selects where the temporary raw files go. On macOS CI, `FFGL_MINIHOST_SOFTWARE=1`
selects the software GL renderer.

## Checked in Resolume

On Windows, Avenue 7.28.1 revision 26535 loaded the DLL as `SGCM`, listed it in the
REST effects endpoint and added it to a layer. A generated six-second H.264 clip
with moving shapes rendered at a 1280x720 composition size. Overlay showed green
boxes and ID chips; Motion matte and Box matte cut out the shapes; Swarm drew the
window collage. All controls accepted live REST changes and read back correctly.
The plugin's current-process log recorded CPU rendering without warnings.

This was a temporary, unpacked copy of Avenue. No installer ran. Only its window
was captured, with PrintWindow. macOS host testing and Arena are unrun; this is a
smoke test, not a long-running show or a saved-project compatibility test.
