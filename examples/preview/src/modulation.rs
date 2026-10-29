//! Expressions on effect parameters: a text box under each slider, like
//! `sin(time / 4)` or `value + bass * 0.5`, evaluated every frame. An empty box leaves
//! the slider in charge; a valid expression replaces the slider's value (which it can
//! still read as `value`).

use egui::{Color32, RichText, Ui};

use crate::controls::Controls;

/// Variables an expression can read, in the order `Sources` stores them.
pub const VARIABLES: &[(&str, &str)] = &[
    ("time", "seconds since start (pauses with Animate)"),
    ("vtime", "video position, seconds"),
    ("progress", "video position, 0–1"),
    ("level", "audio loudness, 0–1"),
    ("bass", "20–250 Hz, 0–1"),
    ("lowmid", "250–1000 Hz, 0–1"),
    ("highmid", "1–4 kHz, 0–1"),
    ("treble", "4 kHz and up, 0–1"),
    // Face tracking; positions are 0–1 across and down the frame.
    ("face", "1 while a face is found, else 0"),
    ("facex", "face centre, x"),
    ("facey", "face centre, y"),
    ("facesize", "face width as a share of the frame width"),
    ("eyelx", "left eye on screen, x"),
    ("eyely", "left eye on screen, y"),
    ("eyerx", "right eye on screen, x"),
    ("eyery", "right eye on screen, y"),
    ("nosex", "nose tip, x"),
    ("nosey", "nose tip, y"),
    ("mouthx", "mouth centre, x"),
    ("mouthy", "mouth centre, y"),
];

/// One frame's variable values, indexed like VARIABLES.
pub type Sources = [f32; 20];

/// The face-tracking variables, in VARIABLES order from `face`.
pub type Face = [f32; 12];

/// Face values before any face is found (or with tracking off): centred guesses, so
/// expressions using them stay sensible, with `face` at 0.
pub const FACE_NONE: Face = [
    0.0, 0.5, 0.5, 0.0, 0.45, 0.45, 0.55, 0.45, 0.5, 0.5, 0.5, 0.6,
];

/// Face values found in the unflipped source, mirrored to match a flipped picture.
/// A horizontal flip also swaps the eyes, so "left" stays the left one on screen.
pub fn flip_face(mut face: Face, x: bool, y: bool) -> Face {
    if x {
        for i in [1, 4, 6, 8, 10] {
            face[i] = 1.0 - face[i];
        }
        face.swap(4, 6);
        face.swap(5, 7);
    }
    if y {
        for i in [2, 5, 7, 9, 11] {
            face[i] = 1.0 - face[i];
        }
    }
    face
}

/// Builds the variable values. Video and audio read 0 without a video.
pub fn sources(time: f64, video: Option<(f64, f64)>, audio: [f32; 5], face: Face) -> Sources {
    let (position, duration) = video.unwrap_or((0.0, 1.0));
    let mut values = [0.0; 20];
    values[0] = time as f32;
    values[1] = position as f32;
    values[2] = (position / duration).clamp(0.0, 1.0) as f32;
    values[3..8].copy_from_slice(&audio);
    values[8..].copy_from_slice(&face);
    values
}

/// A parameter expressions can drive.
pub struct Target {
    pub field: fn(&mut Controls) -> &mut f32,
}

/// Indices into TARGETS, for the panel.
pub mod target {
    pub const BRIGHTNESS: usize = 0;
    pub const CONTRAST: usize = 1;
    pub const SATURATION: usize = 2;
    pub const VIGNETTE_X: usize = 3;
    pub const VIGNETTE_Y: usize = 4;
    pub const VIGNETTE_RADIUS: usize = 5;
    pub const VIGNETTE_SOFTNESS: usize = 6;
    pub const VIGNETTE_AMOUNT: usize = 7;
    pub const BLEND_AMOUNT: usize = 8;
    pub const BLUR_RADIUS: usize = 9;
    pub const FEEDBACK_DECAY: usize = 10;
    pub const FLIP_X: usize = 11;
    pub const FLIP_Y: usize = 12;
}

pub const TARGETS: &[Target] = &[
    Target {
        field: |c| &mut c.brightness,
    },
    Target {
        field: |c| &mut c.contrast,
    },
    Target {
        field: |c| &mut c.saturation,
    },
    Target {
        field: |c| &mut c.vignette_center[0],
    },
    Target {
        field: |c| &mut c.vignette_center[1],
    },
    Target {
        field: |c| &mut c.vignette_radius,
    },
    Target {
        field: |c| &mut c.vignette_softness,
    },
    Target {
        field: |c| &mut c.vignette_amount,
    },
    Target {
        field: |c| &mut c.blend_amount,
    },
    Target {
        field: |c| &mut c.blur_radius,
    },
    Target {
        field: |c| &mut c.feedback_decay,
    },
    Target {
        field: |c| &mut c.flip_x,
    },
    Target {
        field: |c| &mut c.flip_y,
    },
];

/// One parameter's expression box.
#[derive(Default)]
struct Slot {
    text: String,
    compiled: Option<Expr>,
    error: Option<String>,
    /// What it evaluated to last frame, shown under the box.
    last: Option<f32>,
}

impl Slot {
    fn recompile(&mut self) {
        self.last = None;
        (self.compiled, self.error) = if self.text.trim().is_empty() {
            (None, None)
        } else {
            match Expr::parse(&self.text) {
                Ok(e) => (Some(e), None),
                Err(e) => (None, Some(e)),
            }
        };
    }
}

pub struct Expressions {
    slots: Vec<Slot>,
}

impl Default for Expressions {
    fn default() -> Self {
        Self {
            slots: TARGETS.iter().map(|_| Slot::default()).collect(),
        }
    }
}

impl Expressions {
    /// `base` with every valid expression applied. `dt` is the time since the last
    /// frame, for `smooth()`.
    pub fn apply(&mut self, base: &Controls, sources: &Sources, dt: f32) -> Controls {
        let mut controls = base.clone();
        for (slot, target) in self.slots.iter_mut().zip(TARGETS) {
            let Some(expr) = &mut slot.compiled else {
                continue;
            };
            let field = (target.field)(&mut controls);
            let v = expr.eval(&Env {
                sources,
                value: *field,
                dt,
            });
            // NaN or infinity (say, 1/0) keeps the slider's value.
            slot.last = v.is_finite().then_some(v);
            if let Some(v) = slot.last {
                *field = v;
            }
        }
        controls
    }

    /// Forgets `smooth()` history, so an export starts from the source values.
    pub fn reset(&mut self) {
        for expr in self.slots.iter_mut().filter_map(|s| s.compiled.as_mut()) {
            expr.reset();
        }
    }

    #[cfg(test)]
    pub fn set(&mut self, target: usize, text: &str) {
        self.slots[target].text = text.into();
        self.slots[target].recompile();
    }
}

/// The expression box for `target`, drawn under its slider.
pub fn field(ui: &mut Ui, exprs: &mut Expressions, target: usize) {
    let slot = &mut exprs.slots[target];
    let edit = egui::TextEdit::singleline(&mut slot.text)
        .hint_text("expression, e.g. sin(time/4)")
        .font(egui::TextStyle::Monospace)
        .desired_width(f32::INFINITY);
    let edit = if slot.error.is_some() {
        edit.text_color(Color32::from_rgb(230, 120, 90))
    } else {
        edit
    };
    if ui.add(edit).changed() {
        slot.recompile();
    }
    if let Some(error) = &slot.error {
        ui.label(
            RichText::new(error)
                .small()
                .color(Color32::from_rgb(230, 120, 90)),
        );
    } else if slot.compiled.is_some() {
        let shown = slot
            .last
            .map_or("= (not a number; slider used)".into(), |v| {
                format!("= {v:.3}")
            });
        ui.label(RichText::new(shown).small().monospace().weak());
    }
    ui.add_space(4.0);
}

/// The live variable values and what expressions can use.
pub fn help(ui: &mut Ui, sources: &Sources) {
    egui::Grid::new("variables").num_columns(2).show(ui, |ui| {
        for ((name, about), value) in VARIABLES.iter().zip(sources) {
            ui.label(RichText::new(*name).monospace())
                .on_hover_text(*about);
            ui.label(RichText::new(format!("{value:.2}")).monospace().weak());
            ui.end_row();
        }
        ui.label(RichText::new("value").monospace());
        ui.label(RichText::new("the slider").weak());
        ui.end_row();
    });
    ui.label(
        RichText::new(
            "Operators: + - * / % ^, comparisons (< > <= >= == !=) give 1 or 0.\n\
             if(condition, then, otherwise) picks by condition (anything but 0 is true), \
             e.g. if(facesize < 0.2, 0.2, facesize * 2); combine conditions with \
             and(a, b), or(a, b), not(a).\n\
             Functions: sin cos tan asin acos atan atan2 abs sqrt exp log pow min max \
             clamp floor ceil round fract sign mix step smoothstep, and \
             smooth(x, amount) to ease changes (amount 0–1).\nConstants: pi, tau.",
        )
        .small()
        .weak(),
    );
}

// ---- The expression language ----

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug)]
enum Expr {
    Num(f32),
    Var(usize),
    Value,
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(fn(&[f32]) -> f32, Vec<Expr>),
    /// smooth(x, amount), with its running value.
    Smooth(Box<Expr>, Box<Expr>, Option<f32>),
    /// if(condition, then, otherwise). Only the chosen branch runs, so a `smooth()`
    /// in the other one waits rather than tracking.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
}

/// Conditions are true when not 0 (NaN counts as false).
fn truthy(v: f32) -> bool {
    v != 0.0 && !v.is_nan()
}

fn flag(c: bool) -> f32 {
    if c { 1.0 } else { 0.0 }
}

struct Env<'a> {
    sources: &'a Sources,
    value: f32,
    dt: f32,
}

/// name, argument count, implementation.
type Function = (&'static str, usize, fn(&[f32]) -> f32);

const FUNCTIONS: &[Function] = &[
    ("sin", 1, |a| a[0].sin()),
    ("cos", 1, |a| a[0].cos()),
    ("tan", 1, |a| a[0].tan()),
    ("asin", 1, |a| a[0].asin()),
    ("acos", 1, |a| a[0].acos()),
    ("atan", 1, |a| a[0].atan()),
    ("atan2", 2, |a| a[0].atan2(a[1])),
    ("abs", 1, |a| a[0].abs()),
    ("sqrt", 1, |a| a[0].sqrt()),
    ("exp", 1, |a| a[0].exp()),
    ("log", 1, |a| a[0].ln()),
    ("pow", 2, |a| a[0].powf(a[1])),
    ("min", 2, |a| a[0].min(a[1])),
    ("max", 2, |a| a[0].max(a[1])),
    ("clamp", 3, |a| a[0].max(a[1]).min(a[2])),
    ("floor", 1, |a| a[0].floor()),
    ("ceil", 1, |a| a[0].ceil()),
    ("round", 1, |a| a[0].round()),
    ("fract", 1, |a| a[0] - a[0].floor()),
    ("sign", 1, |a| if a[0] == 0.0 { 0.0 } else { a[0].signum() }),
    ("mix", 3, |a| a[0] + (a[1] - a[0]) * a[2]),
    ("lerp", 3, |a| a[0] + (a[1] - a[0]) * a[2]),
    ("step", 2, |a| if a[1] < a[0] { 0.0 } else { 1.0 }),
    ("smoothstep", 3, |a| {
        let t = ((a[2] - a[0]) / (a[1] - a[0])).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }),
    ("and", 2, |a| flag(truthy(a[0]) && truthy(a[1]))),
    ("or", 2, |a| flag(truthy(a[0]) || truthy(a[1]))),
    ("not", 1, |a| flag(!truthy(a[0]))),
];

impl Expr {
    fn parse(text: &str) -> Result<Self, String> {
        let tokens = tokenize(text)?;
        let mut parser = Parser { tokens, pos: 0 };
        let expr = parser.comparison()?;
        match parser.tokens.get(parser.pos) {
            None => Ok(expr),
            Some(t) => Err(format!("unexpected {t}")),
        }
    }

    fn eval(&mut self, env: &Env) -> f32 {
        match self {
            Expr::Num(n) => *n,
            Expr::Var(i) => env.sources[*i],
            Expr::Value => env.value,
            Expr::Neg(e) => -e.eval(env),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(env), b.eval(env));
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    // Always positive, so `time % 4` counts 0..4 like a loop.
                    Op::Rem => a.rem_euclid(b),
                    Op::Pow => a.powf(b),
                    Op::Lt => flag(a < b),
                    Op::Gt => flag(a > b),
                    Op::Le => flag(a <= b),
                    Op::Ge => flag(a >= b),
                    Op::Eq => flag(a == b),
                    Op::Ne => flag(a != b),
                }
            }
            Expr::Call(f, args) => {
                let values: Vec<f32> = args.iter_mut().map(|a| a.eval(env)).collect();
                f(&values)
            }
            Expr::Smooth(x, amount, state) => {
                let x = x.eval(env);
                // Frame-rate independent one-pole smoothing, tuned at 60 fps.
                let keep = amount.eval(env).clamp(0.0, 0.999).powf(env.dt * 60.0);
                let v = match *state {
                    Some(prev) if prev.is_finite() => prev + (x - prev) * (1.0 - keep),
                    _ => x,
                };
                *state = Some(v);
                v
            }
            Expr::If(condition, then, otherwise) => {
                if truthy(condition.eval(env)) {
                    then.eval(env)
                } else {
                    otherwise.eval(env)
                }
            }
        }
    }

    fn reset(&mut self) {
        match self {
            Expr::Num(_) | Expr::Var(_) | Expr::Value => {}
            Expr::Neg(e) => e.reset(),
            Expr::Bin(_, a, b) => {
                a.reset();
                b.reset();
            }
            Expr::Call(_, args) => args.iter_mut().for_each(Expr::reset),
            Expr::Smooth(x, amount, state) => {
                *state = None;
                x.reset();
                amount.reset();
            }
            Expr::If(condition, then, otherwise) => {
                condition.reset();
                then.reset();
                otherwise.reset();
            }
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Token {
    Num(f32),
    Ident(String),
    Op(Op),
    Open,
    Close,
    Comma,
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Token::Num(n) => write!(f, "number {n}"),
            Token::Ident(s) => write!(f, "'{s}'"),
            Token::Op(_) => write!(f, "operator"),
            Token::Open => write!(f, "'('"),
            Token::Close => write!(f, "')'"),
            Token::Comma => write!(f, "','"),
        }
    }
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let two = |op| {
            if next == Some('=') {
                (Token::Op(op), 2)
            } else {
                (Token::Op(op), 1)
            }
        };
        let (token, len) = match c {
            c if c.is_whitespace() => {
                i += 1;
                continue;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let end = (i..chars.len())
                    .find(|&j| !(chars[j].is_ascii_digit() || chars[j] == '.'))
                    .unwrap_or(chars.len());
                let s: String = chars[i..end].iter().collect();
                let n = s.parse().map_err(|_| format!("bad number '{s}'"))?;
                (Token::Num(n), end - i)
            }
            c if c.is_alphabetic() || c == '_' => {
                let end = (i..chars.len())
                    .find(|&j| !(chars[j].is_alphanumeric() || chars[j] == '_'))
                    .unwrap_or(chars.len());
                (Token::Ident(chars[i..end].iter().collect()), end - i)
            }
            '+' => (Token::Op(Op::Add), 1),
            '-' => (Token::Op(Op::Sub), 1),
            '*' if next == Some('*') => (Token::Op(Op::Pow), 2),
            '*' => (Token::Op(Op::Mul), 1),
            '/' => (Token::Op(Op::Div), 1),
            '%' => (Token::Op(Op::Rem), 1),
            '^' => (Token::Op(Op::Pow), 1),
            '(' => (Token::Open, 1),
            ')' => (Token::Close, 1),
            ',' => (Token::Comma, 1),
            '<' if next == Some('=') => two(Op::Le),
            '<' => (Token::Op(Op::Lt), 1),
            '>' if next == Some('=') => two(Op::Ge),
            '>' => (Token::Op(Op::Gt), 1),
            '=' if next == Some('=') => (Token::Op(Op::Eq), 2),
            '!' if next == Some('=') => (Token::Op(Op::Ne), 2),
            c => return Err(format!("unexpected '{c}'")),
        };
        tokens.push(token);
        i += len;
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Result<Token, String> {
        let t = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or("unfinished expression")?;
        self.pos += 1;
        Ok(t)
    }

    fn expect(&mut self, want: Token) -> Result<(), String> {
        match self.next()? {
            t if t == want => Ok(()),
            t => Err(format!("expected {want}, found {t}")),
        }
    }

    /// Binary operators from `ops`, left to right, over `operand`.
    fn binary(
        &mut self,
        ops: &[Op],
        operand: fn(&mut Self) -> Result<Expr, String>,
    ) -> Result<Expr, String> {
        let mut left = operand(self)?;
        while let Some(Token::Op(op)) = self.peek().cloned()
            && ops.contains(&op)
        {
            self.pos += 1;
            left = Expr::Bin(op, Box::new(left), Box::new(operand(self)?));
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        use Op::*;
        self.binary(&[Lt, Gt, Le, Ge, Eq, Ne], Self::sum)
    }

    fn sum(&mut self) -> Result<Expr, String> {
        self.binary(&[Op::Add, Op::Sub], Self::product)
    }

    fn product(&mut self) -> Result<Expr, String> {
        self.binary(&[Op::Mul, Op::Div, Op::Rem], Self::unary)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Token::Op(Op::Sub)) => {
                self.pos += 1;
                Ok(Expr::Neg(Box::new(self.unary()?)))
            }
            Some(Token::Op(Op::Add)) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// `^` binds tighter than unary minus on its left and groups right to left.
    fn power(&mut self) -> Result<Expr, String> {
        let base = self.atom()?;
        if self.peek() == Some(&Token::Op(Op::Pow)) {
            self.pos += 1;
            return Ok(Expr::Bin(Op::Pow, Box::new(base), Box::new(self.unary()?)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.next()? {
            Token::Num(n) => Ok(Expr::Num(n)),
            Token::Open => {
                let e = self.comparison()?;
                self.expect(Token::Close)?;
                Ok(e)
            }
            Token::Ident(name) if self.peek() == Some(&Token::Open) => {
                self.pos += 1;
                let mut args = Vec::new();
                if self.peek() != Some(&Token::Close) {
                    loop {
                        args.push(self.comparison()?);
                        if self.peek() == Some(&Token::Comma) {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Token::Close)?;
                call(&name, args)
            }
            Token::Ident(name) => match name.as_str() {
                "value" => Ok(Expr::Value),
                "pi" => Ok(Expr::Num(std::f32::consts::PI)),
                "tau" => Ok(Expr::Num(std::f32::consts::TAU)),
                // `t` as a short name for time.
                "t" => Ok(Expr::Var(0)),
                _ => VARIABLES
                    .iter()
                    .position(|(v, _)| *v == name)
                    .map(Expr::Var)
                    .ok_or(format!("unknown name '{name}'")),
            },
            t => Err(format!("unexpected {t}")),
        }
    }
}

fn call(name: &str, mut args: Vec<Expr>) -> Result<Expr, String> {
    let arity = |n: usize| {
        if args.len() == n {
            Ok(())
        } else {
            Err(format!(
                "{name}() takes {n} argument{}",
                if n == 1 { "" } else { "s" }
            ))
        }
    };
    if name == "smooth" {
        arity(2)?;
        let amount = args.pop().expect("two arguments");
        let x = args.pop().expect("two arguments");
        return Ok(Expr::Smooth(Box::new(x), Box::new(amount), None));
    }
    if name == "if" {
        arity(3)?;
        let otherwise = args.pop().expect("three arguments");
        let then = args.pop().expect("three arguments");
        let condition = args.pop().expect("three arguments");
        return Ok(Expr::If(
            Box::new(condition),
            Box::new(then),
            Box::new(otherwise),
        ));
    }
    let (_, n, f) = FUNCTIONS
        .iter()
        .find(|(f, _, _)| *f == name)
        .ok_or(format!("unknown function '{name}'"))?;
    arity(*n)?;
    Ok(Expr::Call(*f, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(text: &str, sources: &Sources, value: f32) -> f32 {
        Expr::parse(text).unwrap().eval(&Env {
            sources,
            value,
            dt: 1.0 / 60.0,
        })
    }

    #[test]
    fn arithmetic_follows_the_usual_precedence() {
        let s = sources(8.0, None, [0.0; 5], FACE_NONE);
        assert_eq!(eval("1 + 2 * 3", &s, 0.0), 7.0);
        assert_eq!(eval("(1 + 2) * 3", &s, 0.0), 9.0);
        assert_eq!(eval("-2^2", &s, 0.0), -4.0);
        assert_eq!(eval("2^3^2", &s, 0.0), 512.0);
        assert_eq!(eval("-1 % 4", &s, 0.0), 3.0);
        assert_eq!(eval("time > 5", &s, 0.0), 1.0);
        assert_eq!(eval("sin(time/4)", &s, 0.0), 2.0f32.sin());
        assert_eq!(eval("clamp(value * 10, 0, 1)", &s, 0.5), 1.0);
        assert_eq!(
            eval(
                "mix(0, 10, progress)",
                &sources(0.0, Some((5.0, 10.0)), [0.0; 5], FACE_NONE),
                0.0
            ),
            5.0
        );
    }

    #[test]
    fn flipped_faces_mirror_and_keep_eyes_on_their_side() {
        // Face at x 0.2, left eye 0.15, right eye 0.25, mouth y 0.7.
        let mut face = FACE_NONE;
        face[1] = 0.2;
        face[4] = 0.15;
        face[6] = 0.25;
        face[11] = 0.7;
        let x = flip_face(face, true, false);
        assert!((x[1] - 0.8).abs() < 1e-6);
        // The eye on the left of the flipped picture is the one that was at 0.25.
        assert!(
            (x[4] - 0.75).abs() < 1e-6 && (x[6] - 0.85).abs() < 1e-6,
            "{x:?}"
        );
        assert!(x[4] < x[6]);
        let y = flip_face(face, false, true);
        assert!((y[11] - 0.3).abs() < 1e-6);
        assert_eq!(flip_face(face, false, false), face);
    }

    #[test]
    fn conditions_pick_a_branch() {
        let mut face = FACE_NONE;
        face[3] = 0.1; // facesize
        let small = sources(0.0, None, [0.0; 5], face);
        face[3] = 0.4;
        let big = sources(0.0, None, [0.0; 5], face);
        let e = "if(facesize < 0.2, 0.2, facesize * 2)";
        assert_eq!(eval(e, &small, 0.0), 0.2);
        assert_eq!(eval(e, &big, 0.0), 0.8);
        // Nested, and combined conditions.
        let nested = "if(facesize < 0.2, 1, if(facesize < 0.5, 2, 3))";
        assert_eq!(eval(nested, &big, 0.0), 2.0);
        assert_eq!(eval("and(facesize > 0.3, not(face))", &big, 0.0), 1.0);
        assert_eq!(eval("or(0, 0)", &big, 0.0), 0.0);
        assert!(Expr::parse("if(1, 2)").is_err());
    }

    #[test]
    fn mistakes_are_reported_not_evaluated() {
        for bad in [
            "sin(",
            "1 +",
            "foo",
            "sin(1, 2)",
            "nope(1)",
            "1 2",
            "3 $ 4",
            ")",
        ] {
            assert!(Expr::parse(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn expressions_override_sliders_and_empty_ones_dont() {
        let mut exprs = Expressions::default();
        let base = Controls::default();
        let s = sources(0.0, None, [0.0, 0.8, 0.0, 0.0, 0.0], FACE_NONE);
        exprs.set(target::BRIGHTNESS, "bass - 0.3");
        exprs.set(target::CONTRAST, "  ");
        let out = exprs.apply(&base, &s, 1.0 / 60.0);
        assert!((out.brightness - 0.5).abs() < 1e-6);
        assert_eq!(out.contrast, base.contrast);
        exprs.set(target::BRIGHTNESS, "1 / 0");
        assert_eq!(
            exprs.apply(&base, &s, 1.0 / 60.0).brightness,
            base.brightness
        );
    }

    #[test]
    fn smooth_eases_and_resets() {
        let mut exprs = Expressions::default();
        let base = Controls::default();
        exprs.set(target::BLUR_RADIUS, "smooth(bass * 10, 0.9)");
        let quiet = sources(0.0, None, [0.0; 5], FACE_NONE);
        let loud = sources(0.0, None, [1.0; 5], FACE_NONE);
        assert_eq!(exprs.apply(&base, &quiet, 1.0 / 60.0).blur_radius, 0.0);
        let eased = exprs.apply(&base, &loud, 1.0 / 60.0).blur_radius;
        assert!(eased > 0.5 && eased < 2.0, "{eased}");
        exprs.reset();
        assert_eq!(exprs.apply(&base, &loud, 1.0 / 60.0).blur_radius, 10.0);
    }
}
