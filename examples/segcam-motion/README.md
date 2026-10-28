# segcam Motion, in Rust

segcam's **motion** segmenter as an After Effects effect, ported from segcam's C++ effect
(`segcam/ae`) as an example of a real effect built in this repository. Drop it on a layer
and it finds whatever is moving, then draws it the segcam way.

It is a line-for-line port. Every result the C++ core computes, this one computes the
same way (the same `f32` and `f64` arithmetic in the same order, the same rounding), so
the two render byte-identical frames (see [Checking it](#checking-it)).

| file | |
| --- | --- |
| `src/motion.rs` | the segmenter: background model, morphology, 8-connected labeller |
| `src/render.rs` | settings, the luma grid, the overlay and mattes, and `render`, generic over 8/16/32 bpc |
| `src/swarm.rs` | the swarm: tiles, the window chrome, title layout and the window shadow |
| `src/chrome.rs` | the window chrome baked out of real macOS windows, converted from segcam's `Chrome.h` by `tools/chrome_to_rust.py` |
| `src/font.rs` | the 5x7 bitmap font for labels |
| `src/pixel.rs` | `Argb<u8 / u16 / f32>`, laid out like After Effects' pixels, and strided views of host buffers |
| `src/ae.rs` | the After Effects glue: SmartFX, wide time input, parameters |
| `src/bin/segcam-motion.rs` | the render path as a command-line filter on raw ARGB frames |
| `tests/core.rs` | segcam's core checks, ported |
| `ae-tests/run.py` | renders it in After Effects next to the C++ effect and compares |

## Output modes

- **Overlay**: the layer, each moving region stroked as a box with its ID chip
  (`motion#412.1` is the biggest moving thing in frame 412). Boxes, labels and the box
  colour can be switched.
- **Motion matte**: the layer cut out by the motion mask itself, softened from the grid.
- **Box matte**: the layer cut out by the segments' boxes.
- **Swarm**: segcam's desktop collage. Every segment of every recent frame becomes a frozen
  tile of the frame it was cut from, drawn as the macOS window segcam makes for it (title
  bar, buttons, rounded corners and shadow), newest on top, oldest recycled after
  **Swarm max tiles**.

**Mirror** flips the picture and the boxes, as segcam does for a selfie camera.

## How it renders

After Effects can ask for any frame in any order, on any thread, so the effect keeps no
state between renders. Pre-render checks out the **Background frames** before the current
one (plus **Swarm trail frames** - 1 in Swarm mode), and render hands them to the core one
at a time through `FrameSource::with_frame`. Each frame is held only while the core boils
it down to a luma grid (and, for the swarm, cuts its tiles), then checked back in. So
memory stays flat however many frames it reads. The background is rebuilt for every frame
from those frames, which makes every render deterministic, as After Effects' cache needs.

The C++ effect's README covers how this differs from the live app (no state carried
forever, per-frame IDs); all of it applies here.

This effect runs on the CPU, unlike the repository's GPU effects. Its work is a
connected-component labeller over a 192-column grid, plus drawing about as large as the
frame, fed one frame at a time from After Effects' cache. Uploading 16 to 80 history frames
to the GPU on every render would cost more than the work itself.

## Building

The plugin installs alongside segcam's own: its bundle is `SegcamMotionRust.plugin`, its
match name is `LOFISCIFI segcam Motion Rust`, and it appears as
**Effect ▸ LO.FI.SCI.FI ▸ segcam Motion (Rust)**. Projects saved with one effect don't
switch to the other.

```bash
./examples/segcam-motion/package-mac.sh
```

This builds a universal bundle and `target/vfx-plugin/SegcamMotionRust.pkg`, which installs
into Adobe's MediaCore folder:

```bash
sudo installer -pkg target/vfx-plugin/SegcamMotionRust.pkg -target /
```

`bundle-mac.sh` builds just the bundle. On Windows, `cargo build -p segcam-motion
--release` builds `segcam_motion.dll`; rename it `SegcamMotionRust.aex`.

## Checking it

```bash
cargo test -p segcam-motion
```

runs segcam's core checks:

- A jumping square is found where it lands and where it left.
- A still frame never moves, and the first frame is only a reference.
- 8, 16 and 32 bpc agree, and every frame is read once and in order.
- Mirroring lands on the other side, and the mattes are opaque only on motion.
- The swarm caps its pile, and the window chrome matches the bake.

`segcam-motion` takes the same options as segcam's own CLI, so the two can be compared on
any clip:

```bash
ffmpeg -i in.mov -vf scale=960:540 -f rawvideo -pix_fmt argb - |
  target/release/segcam-motion 960 540 --mode swarm |
  ffmpeg -f rawvideo -pix_fmt argb -s 960x540 -r 30 -i - -pix_fmt yuv420p out.mp4
```

The two match byte for byte in every mode, at sizes up to 1920x1080 and with non-default
options, when the C++ is built with `-ffp-contract=off`. Clang's default fuses multiply-adds
into single FMA instructions that round once instead of twice. Rust never does that, so
against the C++ CLI as normally built, a few soft-edge pixels in Motion matte and Swarm are
one level apart.

With the plugin installed (and segcam's C++ effect, for the comparison):

```bash
python3 examples/segcam-motion/ae-tests/run.py
```

This builds 8, 16 and 32 bpc projects in After Effects, with a comp per output mode for each
effect, renders them with `aerender`, and checks three things:

- The Rust effect matches the C++ one within a level.
- Multi-Frame Rendering on and off give identical frames.
- The movers were found.

`--clip` uses real footage instead of the generated squares.
