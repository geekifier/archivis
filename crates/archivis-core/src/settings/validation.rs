//! Per-setting validators.
//!
//! Each runtime setting declares a `Validator` in the registry. Validators
//! run against an already-canonicalized value, so they only enforce domain
//! constraints (ranges, enum members, non-empty) and not type shape.

use serde_json::Value;

/// Error returned by a `Validator`.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ValidationError {
    pub message: String,
}

impl ValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Function-pointer validator — attached to each `RuntimeSettingMeta`.
pub type Validator = fn(&Value) -> Result<(), ValidationError>;

/// Always-pass validator.
pub fn pass(_: &Value) -> Result<(), ValidationError> {
    Ok(())
}

/// Require a finite 64-bit integer in `[lo, hi]`.
const fn range_i64(lo: i64, hi: i64) -> impl Fn(&Value) -> Result<(), ValidationError> {
    move |v: &Value| {
        let n = v
            .as_i64()
            .ok_or_else(|| ValidationError::new("expected an integer"))?;
        if (lo..=hi).contains(&n) {
            Ok(())
        } else {
            Err(ValidationError::new(format!(
                "must be between {lo} and {hi}"
            )))
        }
    }
}

/// Require a finite 64-bit float in `[lo, hi]`.
fn range_f64(lo: f64, hi: f64) -> impl Fn(&Value) -> Result<(), ValidationError> {
    move |v: &Value| {
        let f = v
            .as_f64()
            .ok_or_else(|| ValidationError::new("expected a number"))?;
        if (lo..=hi).contains(&f) {
            Ok(())
        } else {
            Err(ValidationError::new(format!(
                "must be between {lo} and {hi}"
            )))
        }
    }
}

// Range validators expressed as concrete fn pointers so they can be stored in
// `Validator` slots of each registry entry.
macro_rules! range_i64_fn {
    ($name:ident, $lo:expr, $hi:expr) => {
        pub fn $name(v: &Value) -> Result<(), ValidationError> {
            range_i64($lo, $hi)(v)
        }
    };
}

macro_rules! range_f64_fn {
    ($name:ident, $lo:expr, $hi:expr) => {
        pub fn $name(v: &Value) -> Result<(), ValidationError> {
            range_f64($lo, $hi)(v)
        }
    };
}

range_i64_fn!(port_range, 1, 65535);
range_i64_fn!(rpm_range, 1, 10_000);
range_i64_fn!(concurrency_range, 1, 100);
range_i64_fn!(small_count_range, 1, 100);
range_i64_fn!(bytes_range, 100, 100_000);
range_i64_fn!(debounce_ms_range, 0, 600_000);
range_i64_fn!(poll_interval_range, 1, 86_400);
range_f64_fn!(unit_interval, 0.0, 1.0);

pub fn enum_member(
    options: &'static [&'static str],
) -> impl Fn(&Value) -> Result<(), ValidationError> {
    move |v: &Value| {
        let s = v
            .as_str()
            .ok_or_else(|| ValidationError::new("expected a string"))?;
        if options.contains(&s) {
            Ok(())
        } else {
            Err(ValidationError::new(format!(
                "must be one of: {}",
                options.join(", ")
            )))
        }
    }
}

/// Validate membership against registry options (called from registry code).
pub fn validate_enum(v: &Value, options: &'static [&'static str]) -> Result<(), ValidationError> {
    enum_member(options)(v)
}

pub fn non_empty_string(v: &Value) -> Result<(), ValidationError> {
    match v {
        Value::String(s) if !s.is_empty() => Ok(()),
        Value::String(_) => Err(ValidationError::new("must not be empty")),
        _ => Err(ValidationError::new("expected a string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn port_range_checks_bounds() {
        assert!(port_range(&json!(9514)).is_ok());
        assert!(port_range(&json!(0)).is_err());
        assert!(port_range(&json!(65536)).is_err());
    }

    #[test]
    fn unit_interval_checks_0_to_1() {
        assert!(unit_interval(&json!(0.5)).is_ok());
        assert!(unit_interval(&json!(1.0)).is_ok());
        assert!(unit_interval(&json!(-0.1)).is_err());
        assert!(unit_interval(&json!(1.5)).is_err());
    }
}
