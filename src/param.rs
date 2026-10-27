use std::fmt;

/// Declares one parameter an effect exposes to its host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
}

/// The type and range of a parameter. Values outside a float or int range are clamped on set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Float {
        min: f32,
        max: f32,
        default: f32,
    },
    /// A whole number. Hosts may show small ranges as a menu, labelled by `options`.
    Int {
        min: i32,
        max: i32,
        default: i32,
        options: &'static [&'static str],
    },
    Bool {
        default: bool,
    },
    /// Linear RGBA. Components are not clamped, so HDR colours are allowed.
    Color {
        default: [f32; 4],
    },
    /// A position in normalised frame coordinates: (0, 0) top left, (1, 1) bottom right.
    Point {
        default: [f32; 2],
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Color([f32; 4]),
    Point([f32; 2]),
}

impl ParamSpec {
    pub fn default_value(&self) -> ParamValue {
        match self.kind {
            ParamKind::Float { default, .. } => ParamValue::Float(default),
            ParamKind::Int { default, .. } => ParamValue::Int(default),
            ParamKind::Bool { default } => ParamValue::Bool(default),
            ParamKind::Color { default } => ParamValue::Color(default),
            ParamKind::Point { default } => ParamValue::Point(default),
        }
    }

    fn coerce(&self, value: ParamValue) -> Result<ParamValue, ParamError> {
        match (self.kind, value) {
            (ParamKind::Float { min, max, .. }, ParamValue::Float(v)) => {
                Ok(ParamValue::Float(v.clamp(min, max)))
            }
            (ParamKind::Int { min, max, .. }, ParamValue::Int(v)) => {
                Ok(ParamValue::Int(v.clamp(min, max)))
            }
            (ParamKind::Bool { .. }, ParamValue::Bool(v)) => Ok(ParamValue::Bool(v)),
            (ParamKind::Color { .. }, ParamValue::Color(v)) => Ok(ParamValue::Color(v)),
            (ParamKind::Point { .. }, ParamValue::Point(v)) => Ok(ParamValue::Point(v)),
            _ => Err(ParamError::TypeMismatch(self.name)),
        }
    }
}

/// The current values for one effect instance, checked against its specs.
#[derive(Clone, Debug)]
pub struct ParamValues {
    specs: Vec<ParamSpec>,
    values: Vec<ParamValue>,
}

impl ParamValues {
    pub fn new(specs: &[ParamSpec]) -> Self {
        Self {
            specs: specs.to_vec(),
            values: specs.iter().map(ParamSpec::default_value).collect(),
        }
    }

    /// Sets a value, clamping numbers into range. Errors on an unknown name or a type mismatch.
    pub fn set(&mut self, name: &str, value: ParamValue) -> Result<(), ParamError> {
        let index = self
            .position(name)
            .ok_or_else(|| ParamError::Unknown(name.to_owned()))?;
        self.values[index] = self.specs[index].coerce(value)?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<ParamValue> {
        self.position(name).map(|i| self.values[i])
    }

    // Typed readers. Each panics on a missing name or wrong type, since effects
    // only ask for parameters they declared themselves.

    pub fn float(&self, name: &str) -> f32 {
        match self.get(name) {
            Some(ParamValue::Float(v)) => v,
            other => panic!("expected float parameter `{name}`, found {other:?}"),
        }
    }

    pub fn int(&self, name: &str) -> i32 {
        match self.get(name) {
            Some(ParamValue::Int(v)) => v,
            other => panic!("expected int parameter `{name}`, found {other:?}"),
        }
    }

    pub fn bool(&self, name: &str) -> bool {
        match self.get(name) {
            Some(ParamValue::Bool(v)) => v,
            other => panic!("expected bool parameter `{name}`, found {other:?}"),
        }
    }

    pub fn color(&self, name: &str) -> [f32; 4] {
        match self.get(name) {
            Some(ParamValue::Color(v)) => v,
            other => panic!("expected color parameter `{name}`, found {other:?}"),
        }
    }

    pub fn point(&self, name: &str) -> [f32; 2] {
        match self.get(name) {
            Some(ParamValue::Point(v)) => v,
            other => panic!("expected point parameter `{name}`, found {other:?}"),
        }
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.specs.iter().position(|s| s.name == name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamError {
    Unknown(String),
    TypeMismatch(&'static str),
}

impl fmt::Display for ParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamError::Unknown(name) => write!(f, "unknown parameter `{name}`"),
            ParamError::TypeMismatch(name) => write!(f, "wrong value type for parameter `{name}`"),
        }
    }
}

impl std::error::Error for ParamError {}

#[cfg(test)]
mod tests {
    use super::*;

    const SPECS: &[ParamSpec] = &[
        ParamSpec {
            name: "gain",
            kind: ParamKind::Float {
                min: 0.0,
                max: 2.0,
                default: 1.0,
            },
        },
        ParamSpec {
            name: "steps",
            kind: ParamKind::Int {
                min: 1,
                max: 8,
                default: 2,
                options: &[],
            },
        },
        ParamSpec {
            name: "invert",
            kind: ParamKind::Bool { default: false },
        },
    ];

    #[test]
    fn starts_at_defaults() {
        let p = ParamValues::new(SPECS);
        assert_eq!(p.float("gain"), 1.0);
        assert_eq!(p.get("steps"), Some(ParamValue::Int(2)));
        assert_eq!(p.get("invert"), Some(ParamValue::Bool(false)));
    }

    #[test]
    fn clamps_numbers_into_range() {
        let mut p = ParamValues::new(SPECS);
        p.set("gain", ParamValue::Float(5.0)).unwrap();
        assert_eq!(p.float("gain"), 2.0);
        p.set("steps", ParamValue::Int(-3)).unwrap();
        assert_eq!(p.get("steps"), Some(ParamValue::Int(1)));
    }

    #[test]
    fn rejects_unknown_names_and_wrong_types() {
        let mut p = ParamValues::new(SPECS);
        assert_eq!(
            p.set("missing", ParamValue::Float(0.0)),
            Err(ParamError::Unknown("missing".into()))
        );
        assert_eq!(
            p.set("gain", ParamValue::Bool(true)),
            Err(ParamError::TypeMismatch("gain"))
        );
    }
}
