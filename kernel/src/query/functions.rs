// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Built-in functions for EigenQL expressions.

use crate::ontology::resource::Value;
use crate::query::error::QueryError;

/// Evaluate a built-in function call.
pub fn call_function(name: &str, args: &[Value]) -> Result<Value, QueryError> {
    match name {
        "DATE" => fn_date(args),
        "TIMESTAMP" => fn_timestamp(args),
        "REGEX" => fn_regex(args),
        "LENGTH" => fn_length(args),
        "CONTAINS" => fn_contains(args),
        "CONCAT" => fn_concat(args),
        "UNIT" => fn_unit(args),
        "DIMENSION" => fn_dimension(args),
        _ => Err(QueryError::evaluation(format!("unknown function: {name}"))),
    }
}

fn fn_date(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 1 {
        return Err(QueryError::evaluation("DATE requires 1 argument"));
    }
    let s = args[0]
        .as_str()
        .ok_or_else(|| QueryError::evaluation("DATE argument must be a string"))?;
    // Validate date format
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    if !re.is_match(s) {
        return Err(QueryError::evaluation(format!(
            "invalid date format: '{s}'"
        )));
    }
    Ok(Value::String(s.to_string()))
}

fn fn_timestamp(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 1 {
        return Err(QueryError::evaluation("TIMESTAMP requires 1 argument"));
    }
    let s = args[0]
        .as_str()
        .ok_or_else(|| QueryError::evaluation("TIMESTAMP argument must be a string"))?;
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$")
        .unwrap();
    if !re.is_match(s) {
        return Err(QueryError::evaluation(format!(
            "invalid datetime format: '{s}'"
        )));
    }
    Ok(Value::String(s.to_string()))
}

fn fn_regex(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 1 {
        return Err(QueryError::evaluation("REGEX requires 1 argument"));
    }
    let s = args[0]
        .as_str()
        .ok_or_else(|| QueryError::evaluation("REGEX argument must be a string"))?;
    regex::Regex::new(s).map_err(|e| QueryError::evaluation(format!("invalid regex: {e}")))?;
    Ok(Value::String(s.to_string()))
}

/// `UNIT(s)` — validates a canonical unit string and returns it.
///
/// D93 carries a unit as its canonical STRING rather than as an embedded resource, for three
/// reasons recorded there: an embedded value adds a `core:mentions` edge per quantity occurrence,
/// it costs term size, and a string keeps one carrier rule shared with `core:rational`. This
/// function and `DIMENSION` below are what pay for that — the query layer decomposes the string, so
/// nothing is lost by not storing it decomposed.
///
/// `DATE` above is the same shape: a composite value (year, month, day) carried as canonical text
/// and given meaning by a function.
fn fn_unit(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 1 {
        return Err(QueryError::evaluation("UNIT requires 1 argument"));
    }
    let s = args[0]
        .as_str()
        .ok_or_else(|| QueryError::evaluation("UNIT argument must be a string"))?;
    let u = crate::units::Unit::parse_canonical(s)
        .map_err(|e| QueryError::evaluation(format!("UNIT: {e}")))?;
    Ok(Value::String(u.to_canonical_string()))
}

/// `DIMENSION(u)` — the unit with its kinds dropped, leaving the physical dimension.
///
/// `DIMENSION('angle')` is `'1'`: `rad` and the plain dimensionless unit are distinct VALUES, which
/// is why the kind axis exists, and this is the projection that groups them again. Two units are
/// commensurable — convertible by scaling — exactly when their dimensions are equal, so
/// `DIMENSION(a) = DIMENSION(b)` is that test and no separate function is needed for it.
fn fn_dimension(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 1 {
        return Err(QueryError::evaluation("DIMENSION requires 1 argument"));
    }
    let s = args[0]
        .as_str()
        .ok_or_else(|| QueryError::evaluation("DIMENSION argument must be a string"))?;
    let u = crate::units::Unit::parse_canonical(s)
        .map_err(|e| QueryError::evaluation(format!("DIMENSION: {e}")))?;
    Ok(Value::String(u.dimension_only().to_canonical_string()))
}

fn fn_length(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 1 {
        return Err(QueryError::evaluation("LENGTH requires 1 argument"));
    }
    match &args[0] {
        Value::String(s) => Ok(Value::Integer(s.chars().count() as i64)),
        Value::Array(arr) => Ok(Value::Integer(arr.len() as i64)),
        _ => Err(QueryError::evaluation(
            "LENGTH argument must be a string or array",
        )),
    }
}

fn fn_contains(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 2 {
        return Err(QueryError::evaluation("CONTAINS requires 2 arguments"));
    }
    let arr = match &args[0] {
        Value::Array(a) => a,
        _ => {
            return Err(QueryError::evaluation(
                "CONTAINS first argument must be an array",
            ))
        }
    };
    let needle = &args[1];
    let found = arr.iter().any(|v| values_equal(v, needle));
    Ok(Value::Boolean(found))
}

fn fn_concat(args: &[Value]) -> Result<Value, QueryError> {
    if args.len() != 2 {
        return Err(QueryError::evaluation("CONCAT requires 2 arguments"));
    }
    let a = match &args[0] {
        Value::Array(a) => a.clone(),
        _ => {
            return Err(QueryError::evaluation(
                "CONCAT first argument must be an array",
            ))
        }
    };
    let b = match &args[1] {
        Value::Array(b) => b.clone(),
        _ => {
            return Err(QueryError::evaluation(
                "CONCAT second argument must be an array",
            ))
        }
    };
    let mut result = a;
    result.extend(b);
    Ok(Value::Array(result))
}

/// Compare two values for equality (used in CONTAINS, =, etc.)
pub fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Integer(a), Value::Float(b)) => (*a as f64) == *b,
        (Value::Float(a), Value::Integer(b)) => *a == (*b as f64),
        // The IRI-identity special case that used to sit here is gone with `ResourceRef`.
        // It existed so a `ResourceRef` to `urn:x:y` compared equal to a `String` holding
        // `"urn:x:y"` — "without this, equi-joins over cross-referenced resources silently
        // miss bindings depending on which `Value` variant the loader happened to use".
        // There is now one variant, so the string arm above IS that comparison, and derived
        // `PartialEq` agrees with this function instead of contradicting it.
        _ => false,
    }
}

/// Compare two values for ordering (used in <, >, MIN, MAX, ORDER BY)
pub fn values_compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        (Value::Integer(a), Value::Integer(b)) => Some(a.cmp(b)),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Boolean(a), Value::Boolean(b)) => Some(a.cmp(b)),
        (Value::Integer(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
        (Value::Float(a), Value::Integer(b)) => a.partial_cmp(&(*b as f64)),
        _ => None,
    }
}

/// Check if a string matches a LIKE pattern (SQL-style: % = any, _ = single char)
pub fn like_match(value: &str, pattern: &str) -> bool {
    like_match_inner(value.as_bytes(), pattern.as_bytes())
}

fn like_match_inner(value: &[u8], pattern: &[u8]) -> bool {
    if pattern.is_empty() {
        return value.is_empty();
    }
    match pattern[0] {
        b'%' => {
            // % matches zero or more characters
            for i in 0..=value.len() {
                if like_match_inner(&value[i..], &pattern[1..]) {
                    return true;
                }
            }
            false
        }
        b'_' => {
            // _ matches exactly one character
            !value.is_empty() && like_match_inner(&value[1..], &pattern[1..])
        }
        ch => !value.is_empty() && value[0] == ch && like_match_inner(&value[1..], &pattern[1..]),
    }
}

/// Extract a numeric value as f64 for arithmetic.
pub fn to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_exact() {
        assert!(like_match("hello", "hello"));
        assert!(!like_match("hello", "world"));
    }

    #[test]
    fn like_percent() {
        assert!(like_match("hello world", "hello%"));
        assert!(like_match("hello world", "%world"));
        assert!(like_match("hello world", "%lo w%"));
        assert!(like_match("", "%"));
    }

    #[test]
    fn like_underscore() {
        assert!(like_match("hello", "hell_"));
        assert!(!like_match("hello", "hel_"));
        assert!(like_match("hello", "_ello"));
    }

    #[test]
    fn length_string() {
        let result = call_function("LENGTH", &[Value::String("hello".into())]).unwrap();
        assert_eq!(result.as_integer(), Some(5));
    }

    #[test]
    fn length_array() {
        let result = call_function(
            "LENGTH",
            &[Value::Array(vec![Value::Integer(1), Value::Integer(2)])],
        )
        .unwrap();
        assert_eq!(result.as_integer(), Some(2));
    }

    #[test]
    fn contains_found() {
        let result = call_function(
            "CONTAINS",
            &[
                Value::Array(vec![
                    Value::Integer(1),
                    Value::Integer(2),
                    Value::Integer(3),
                ]),
                Value::Integer(2),
            ],
        )
        .unwrap();
        assert_eq!(result.as_boolean(), Some(true));
    }

    #[test]
    fn contains_not_found() {
        let result = call_function(
            "CONTAINS",
            &[Value::Array(vec![Value::Integer(1)]), Value::Integer(5)],
        )
        .unwrap();
        assert_eq!(result.as_boolean(), Some(false));
    }

    #[test]
    fn concat_arrays() {
        let result = call_function(
            "CONCAT",
            &[
                Value::Array(vec![Value::Integer(1)]),
                Value::Array(vec![Value::Integer(2)]),
            ],
        )
        .unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[test]
    fn values_compare_integers() {
        assert_eq!(
            values_compare(&Value::Integer(1), &Value::Integer(2)),
            Some(std::cmp::Ordering::Less)
        );
    }

    fn call(name: &str, args: &[&str]) -> Result<Value, QueryError> {
        let owned: Vec<Value> = args.iter().map(|a| Value::String(a.to_string())).collect();
        call_function(name, &owned)
    }

    /// `UNIT` validates and returns the canonical form — the `DATE` shape, on a unit.
    #[test]
    fn unit_validates_a_canonical_string() {
        assert_eq!(
            call("UNIT", &["s^-1\u{b7}m^2"]).unwrap(),
            Value::String("s^-1\u{b7}m^2".to_string())
        );
        assert_eq!(
            call("UNIT", &["1"]).unwrap(),
            Value::String("1".to_string())
        );
    }

    /// A non-canonical spelling is an ERROR, not a silent normalisation — the query layer holds the
    /// same line as the parser and the serde boundary.
    #[test]
    fn unit_refuses_a_non_canonical_string() {
        for bad in ["m\u{b7}s^-1\u{b7}m", "m^1", "metre", "M"] {
            assert!(call("UNIT", &[bad]).is_err(), "{bad:?} should have failed");
        }
        assert!(call_function("UNIT", &[Value::Integer(3)]).is_err());
        assert!(call_function("UNIT", &[]).is_err());
    }

    /// `DIMENSION` groups what the kind axis separates: `rad` and `1` share a dimension.
    #[test]
    fn dimension_drops_the_kind() {
        assert_eq!(
            call("DIMENSION", &["angle"]).unwrap(),
            Value::String("1".to_string())
        );
        assert_eq!(
            call("DIMENSION", &["1"]).unwrap(),
            Value::String("1".to_string())
        );
        // Equal dimensions is the commensurability test, so no separate function exists for it.
        assert_eq!(
            call("DIMENSION", &["angle"]).unwrap(),
            call("DIMENSION", &["1"]).unwrap()
        );
        // And a dimensioned unit is unchanged.
        assert_eq!(
            call("DIMENSION", &["m"]).unwrap(),
            Value::String("m".to_string())
        );
    }
}
