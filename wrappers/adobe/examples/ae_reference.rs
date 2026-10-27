//! Renders frames through the plugin's pipeline without After Effects, for the test
//! harness in `ae-tests/`. It compares After Effects' renders against this output.
//!
//! Usage: ae_reference <job-file>
//!
//! The job file holds one frame per block, separated by blank lines. Each line is a key
//! followed by space-separated values:
//!
//!   size 128 96                 width, height
//!   input in.rgba               raw 8-bit straight RGBA, width x height
//!   layer layer.rgba            optional blend layer, same size
//!   output out.rgbaf            written as raw f32 straight RGBA, unclamped
//!   time 0.5                    seconds
//!   linear 1
//!   color 0.1 1.2 0.8           brightness contrast saturation
//!   vignette cx cy radius softness r g b a amount
//!   blend 1 0.6                 mode (0-based) amount
//!   blur 6 1 0 0                radius vertical_scale direction (0-based) grow
//!   repeat 2                    run the chain this many times (stacked instances)
//!
//! Missing stages are off, matching the plugin's checkboxes.

use std::fs;
use std::process::ExitCode;

use vfx_adobe::reference::{Blur, Settings, Vignette, render};

struct Frame {
    width: u32,
    height: u32,
    input: String,
    layer: Option<String>,
    output: String,
    time: f64,
    repeat: u32,
    settings: Settings,
}

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: ae_reference <job-file>");
        return ExitCode::FAILURE;
    };
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    for (n, block) in text
        .split("\n\n")
        .filter(|b| !b.trim().is_empty())
        .enumerate()
    {
        if let Err(e) = parse(block).and_then(|f| run(&f)) {
            eprintln!("frame {}: {e}", n + 1);
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn parse(block: &str) -> Result<Frame, String> {
    let mut frame = Frame {
        width: 0,
        height: 0,
        input: String::new(),
        layer: None,
        output: String::new(),
        time: 0.0,
        repeat: 1,
        settings: Settings {
            linear: false,
            color: None,
            vignette: None,
            blend_mode: 0,
            blend_amount: 0.0,
            blur: None,
        },
    };
    for line in block.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        let nums = || -> Result<Vec<f32>, String> {
            rest.split_whitespace()
                .map(|v| {
                    v.parse::<f32>()
                        .map_err(|_| format!("bad number in `{line}`"))
                })
                .collect()
        };
        let want = |n: usize| -> Result<Vec<f32>, String> {
            let v = nums()?;
            if v.len() == n {
                Ok(v)
            } else {
                Err(format!("`{key}` takes {n} values"))
            }
        };
        let s = &mut frame.settings;
        match key {
            "size" => {
                let v = want(2)?;
                (frame.width, frame.height) = (v[0] as u32, v[1] as u32);
            }
            "input" => frame.input = rest.to_owned(),
            "layer" => frame.layer = Some(rest.to_owned()),
            "output" => frame.output = rest.to_owned(),
            "time" => frame.time = f64::from(want(1)?[0]),
            "repeat" => frame.repeat = want(1)?[0] as u32,
            "linear" => s.linear = want(1)?[0] != 0.0,
            "color" => {
                let v = want(3)?;
                s.color = Some([v[0], v[1], v[2]]);
            }
            "vignette" => {
                let v = want(9)?;
                s.vignette = Some(Vignette {
                    center_px: [v[0], v[1]],
                    radius: v[2],
                    softness: v[3],
                    color: [v[4], v[5], v[6], v[7]],
                    amount: v[8],
                });
            }
            "blend" => {
                let v = want(2)?;
                (s.blend_mode, s.blend_amount) = (v[0] as i32, v[1]);
            }
            "blur" => {
                let v = want(4)?;
                // Same rule as the plugin: a zero radius switches the stage off.
                s.blur = (v[0] > 0.0).then_some(Blur {
                    radius: v[0],
                    vertical_scale: v[1],
                    direction: v[2] as i32,
                    grow: v[3] != 0.0,
                });
            }
            _ => return Err(format!("unknown key `{key}`")),
        }
    }
    if frame.width == 0 || frame.height == 0 || frame.input.is_empty() || frame.output.is_empty() {
        return Err("needs size, input and output".into());
    }
    Ok(frame)
}

fn run(frame: &Frame) -> Result<(), String> {
    let read = |path: &str| -> Result<Vec<f32>, String> {
        let bytes = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        if bytes.len() != (frame.width * frame.height * 4) as usize {
            return Err(format!(
                "{path}: expected {}x{} RGBA",
                frame.width, frame.height
            ));
        }
        Ok(bytes.iter().map(|&b| f32::from(b) / 255.0).collect())
    };
    let mut pixels = read(&frame.input)?;
    let layer = frame.layer.as_deref().map(read).transpose()?;
    for _ in 0..frame.repeat {
        pixels = render(
            &frame.settings,
            frame.width,
            frame.height,
            &pixels,
            layer.as_deref(),
            frame.time,
        )
        .ok_or("no GPU adapter")?;
    }
    // Unclamped f32, so the harness can quantise the way each bit depth does: integer
    // depths clip before premultiplying, 32 bpc keeps values above 1.0 until output.
    fs::write(&frame.output, bytemuck::cast_slice::<f32, u8>(&pixels))
        .map_err(|e| format!("{}: {e}", frame.output))
}
