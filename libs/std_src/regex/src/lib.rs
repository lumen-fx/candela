//! The native library behind `std/regex`.
//!
//! Every export takes the pattern as text and compiles it once: the compiled
//! form is kept in a small table by pattern, so a `Regex` the program holds
//! is only its pattern and nothing needs freeing. Results that are lists
//! cross as JSON text, which the module decodes.

use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::{Mutex, OnceLock};

use regex_lite::Regex;

/// How many compiled patterns are kept before the table starts over.
const KEPT: usize = 256;

fn compiled(pattern: &str) -> Result<Regex, String> {
    static TABLE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let mut table = TABLE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(re) = table.get(pattern) {
        return Ok(re.clone());
    }
    let re = Regex::new(pattern).map_err(|e| e.to_string())?;
    if table.len() >= KEPT {
        table.clear();
    }
    table.insert(pattern.to_owned(), re.clone());
    Ok(re)
}

/// Reads the two strings of a call.
///
/// # Safety
///
/// Both are NUL-terminated strings the runtime passes.
unsafe fn args(pattern: *const c_char, text: *const c_char) -> (String, String) {
    unsafe { (candela_std_sys::arg(pattern), candela_std_sys::arg(text)) }
}

/// An empty string when `pattern` compiles, and the reason when it does not.
///
/// # Safety
///
/// `pattern` is a NUL-terminated string the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_regex_check(pattern: *const c_char) -> *const c_char {
    let pattern = unsafe { candela_std_sys::arg(pattern) };
    candela_std_sys::ret(&compiled(&pattern).err().unwrap_or_default())
}

/// 1 when `pattern` matches somewhere in `text`.
///
/// # Safety
///
/// Both are NUL-terminated strings the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_regex_is_match(
    pattern: *const c_char,
    text: *const c_char,
) -> i64 {
    let (pattern, text) = unsafe { args(pattern, text) };
    i64::from(compiled(&pattern).is_ok_and(|re| re.is_match(&text)))
}

/// The first match as a JSON string, or `null`.
///
/// # Safety
///
/// Both are NUL-terminated strings the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_regex_find(
    pattern: *const c_char,
    text: *const c_char,
) -> *const c_char {
    let (pattern, text) = unsafe { args(pattern, text) };
    let found = compiled(&pattern)
        .ok()
        .and_then(|re| re.find(&text).map(|m| m.as_str().to_owned()));
    candela_std_sys::ret(&serde_json::json!(found).to_string())
}

/// Every match that does not overlap the one before, as a JSON array.
///
/// # Safety
///
/// Both are NUL-terminated strings the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_regex_find_all(
    pattern: *const c_char,
    text: *const c_char,
) -> *const c_char {
    let (pattern, text) = unsafe { args(pattern, text) };
    let all: Vec<String> = compiled(&pattern)
        .map(|re| re.find_iter(&text).map(|m| m.as_str().to_owned()).collect())
        .unwrap_or_default();
    candela_std_sys::ret(&serde_json::json!(all).to_string())
}

/// The first match's groups as a JSON array, the whole match first and a
/// group that took part in no match as an empty string; `null` for no match.
///
/// # Safety
///
/// Both are NUL-terminated strings the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_regex_captures(
    pattern: *const c_char,
    text: *const c_char,
) -> *const c_char {
    let (pattern, text) = unsafe { args(pattern, text) };
    let groups: Option<Vec<String>> = compiled(&pattern).ok().and_then(|re| {
        re.captures(&text).map(|caps| {
            caps.iter()
                .map(|g| g.map_or_else(String::new, |m| m.as_str().to_owned()))
                .collect()
        })
    });
    candela_std_sys::ret(&serde_json::json!(groups).to_string())
}

/// `text` with every match replaced by `with`, in which `$1` or `${name}`
/// stands for a group.
///
/// # Safety
///
/// Every pointer is a NUL-terminated string the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_regex_replace_all(
    pattern: *const c_char,
    text: *const c_char,
    with: *const c_char,
) -> *const c_char {
    let (pattern, text) = unsafe { args(pattern, text) };
    let with = unsafe { candela_std_sys::arg(with) };
    let out = compiled(&pattern).map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, with.as_str()).into_owned(),
    );
    candela_std_sys::ret(&out)
}

/// The pieces of `text` between matches, as a JSON array.
///
/// # Safety
///
/// Both are NUL-terminated strings the runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_regex_split(
    pattern: *const c_char,
    text: *const c_char,
) -> *const c_char {
    let (pattern, text) = unsafe { args(pattern, text) };
    let pieces: Vec<String> = compiled(&pattern)
        .map(|re| re.split(&text).map(str::to_owned).collect())
        .unwrap_or_else(|_| vec![text.clone()]);
    candela_std_sys::ret(&serde_json::json!(pieces).to_string())
}
