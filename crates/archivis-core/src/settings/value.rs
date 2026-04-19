//! Canonical JSON representation and comparison for setting values.
//!
//! DB rows store `TEXT`, but the in-memory representation is `serde_json::Value`
//! canonicalized by the registry's declared type. This removes drift between
//! `"2000"` (legacy string) vs `2000` (int) vs `2000.0` (float-shaped int) and
//! `"false"` vs `false`.

use serde_json::Value;

use super::registry::SettingType;
use super::validation::ValidationError;

/// The in-memory representation of any setting value.
pub type SettingValue = Value;

/// A provider of a setting's default value.
///
/// `serde_json::Value` cannot be stored in a `const`, so defaults are exposed
/// through a zero-argument fn pointer that returns the canonical default.
pub type SettingDefault = fn() -> SettingValue;

/// Canonicalize `v` against the declared `ty`.
///
/// * Integer types accept `"123"`, `123`, and `123.0` (when exact).
/// * Bool accepts `true`, `false`, `"true"`, `"false"`.
/// * Float accepts numeric types and numeric-string.
/// * `OptionalString` treats `null` as `None`.
pub fn canonicalize(ty: SettingType, v: &Value) -> Result<Value, ValidationError> {
    match ty {
        SettingType::String | SettingType::Select => match v {
            Value::String(_) => Ok(v.clone()),
            _ => Err(ValidationError::new("expected a string")),
        },
        SettingType::OptionalString => match v {
            Value::String(_) | Value::Null => Ok(v.clone()),
            _ => Err(ValidationError::new("expected a string or null")),
        },
        SettingType::Bool => match v {
            Value::Bool(_) => Ok(v.clone()),
            Value::String(s) => match s.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(ValidationError::new("expected a boolean")),
            },
            _ => Err(ValidationError::new("expected a boolean")),
        },
        SettingType::Integer => canonicalize_integer(v),
        SettingType::Float => canonicalize_float(v),
    }
}

fn canonicalize_integer(v: &Value) -> Result<Value, ValidationError> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return Ok(Value::Number(i.into()));
            }
            if let Some(f) = n.as_f64() {
                #[allow(clippy::cast_possible_truncation)]
                let rounded = f as i64;
                #[allow(clippy::cast_precision_loss)]
                if (rounded as f64 - f).abs() < f64::EPSILON {
                    return Ok(Value::Number(rounded.into()));
                }
            }
            Err(ValidationError::new("expected an integer"))
        }
        Value::String(s) => s
            .parse::<i64>()
            .map(|i| Value::Number(i.into()))
            .map_err(|_| ValidationError::new("expected an integer")),
        _ => Err(ValidationError::new("expected an integer")),
    }
}

fn canonicalize_float(v: &Value) -> Result<Value, ValidationError> {
    match v {
        Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| ValidationError::new("expected a number")),
        Value::String(s) => s
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| ValidationError::new("expected a number")),
        _ => Err(ValidationError::new("expected a number")),
    }
}

/// Compare two values for logical equality under the given type.
///
/// Numeric equality ignores integer-vs-float shape; `"false"` equals `false`
/// once both are canonicalized.
pub fn values_equal(ty: SettingType, a: &Value, b: &Value) -> bool {
    match (canonicalize(ty, a), canonicalize(ty, b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integer_accepts_string_number_and_exact_float() {
        assert_eq!(
            canonicalize(SettingType::Integer, &json!("2000")).unwrap(),
            json!(2000)
        );
        assert_eq!(
            canonicalize(SettingType::Integer, &json!(2000.0)).unwrap(),
            json!(2000)
        );
        assert_eq!(
            canonicalize(SettingType::Integer, &json!(2000)).unwrap(),
            json!(2000)
        );
        assert!(canonicalize(SettingType::Integer, &json!(2000.5)).is_err());
    }

    #[test]
    fn bool_accepts_string_and_native() {
        assert_eq!(
            canonicalize(SettingType::Bool, &json!("false")).unwrap(),
            json!(false)
        );
        assert_eq!(
            canonicalize(SettingType::Bool, &json!(true)).unwrap(),
            json!(true)
        );
        assert!(canonicalize(SettingType::Bool, &json!("yes")).is_err());
    }

    #[test]
    fn optional_string_accepts_null() {
        assert_eq!(
            canonicalize(SettingType::OptionalString, &json!(null)).unwrap(),
            json!(null)
        );
        assert_eq!(
            canonicalize(SettingType::OptionalString, &json!("x")).unwrap(),
            json!("x")
        );
    }

    #[test]
    fn values_equal_ignores_shape() {
        assert!(values_equal(
            SettingType::Integer,
            &json!(2000),
            &json!("2000")
        ));
        assert!(values_equal(
            SettingType::Integer,
            &json!(2000.0),
            &json!(2000)
        ));
        assert!(values_equal(
            SettingType::Bool,
            &json!("false"),
            &json!(false)
        ));
    }

    #[test]
    fn integer_rejects_negative_overflow() {
        let max_plus_one = "9223372036854775808"; // i64::MAX + 1
        assert!(canonicalize(
            SettingType::Integer,
            &Value::String(max_plus_one.to_string())
        )
        .is_err());
    }
}
