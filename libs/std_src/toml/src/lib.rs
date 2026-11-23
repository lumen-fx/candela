//! The native library behind `std/toml`.
//!
//! Values cross the C boundary as JSON text: `{"value": ...}` for a result,
//! `{"error": "why"}` for a failure, which the module reads with
//! `json::parse`. A TOML date or time becomes its text, since candela has no
//! date type.

use std::ffi::c_char;

/// TOML's value as the JSON a candela program reads: tables to objects,
/// arrays to arrays, dates and times to their text.
fn to_json(value: toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s),
        toml::Value::Integer(i) => serde_json::Value::from(i),
        toml::Value::Float(f) => serde_json::Value::from(f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(to_json).collect())
        }
        toml::Value::Table(table) => {
            serde_json::Value::Object(table.into_iter().map(|(k, v)| (k, to_json(v))).collect())
        }
    }
}

/// A JSON value as TOML's; `null` has no TOML form.
fn from_json(value: serde_json::Value) -> Result<toml::Value, String> {
    Ok(match value {
        serde_json::Value::Null => return Err(String::from("toml has no null")),
        serde_json::Value::Bool(b) => toml::Value::Boolean(b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => toml::Value::Integer(i),
            None => toml::Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(s) => toml::Value::String(s),
        serde_json::Value::Array(items) => {
            toml::Value::Array(items.into_iter().map(from_json).collect::<Result<_, _>>()?)
        }
        serde_json::Value::Object(map) => toml::Value::Table(
            map.into_iter()
                .map(|(k, v)| Ok((k, from_json(v)?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

fn answer(outcome: Result<serde_json::Value, String>) -> *const c_char {
    let text = match outcome {
        Ok(value) => serde_json::json!({"value": value}),
        Err(why) => serde_json::json!({"error": why}),
    };
    candela_std_sys::ret(&text.to_string())
}

/// Parses TOML `text` into its value, as `{"value": ...}` or `{"error": ...}`.
///
/// # Safety
///
/// `text` is a NUL-terminated string the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_toml_parse(text: *const c_char) -> *const c_char {
    let text = unsafe { candela_std_sys::arg(text) };
    answer(
        text.parse::<toml::Table>()
            .map(|table| to_json(toml::Value::Table(table)))
            .map_err(|e| e.to_string().trim_end().to_owned()),
    )
}

/// Writes the JSON object `json` as TOML text, as `{"value": "..."}` or
/// `{"error": ...}`.
///
/// # Safety
///
/// `json` is a NUL-terminated string the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_toml_stringify(json: *const c_char) -> *const c_char {
    let json = unsafe { candela_std_sys::arg(json) };
    answer((|| {
        let value: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        let toml::Value::Table(table) = from_json(value)? else {
            return Err(String::from("only a map has a TOML form at the top"));
        };
        toml::to_string(&table)
            .map(serde_json::Value::String)
            .map_err(|e| e.to_string())
    })())
}
