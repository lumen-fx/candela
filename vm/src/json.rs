//! json parse and stringify over the runtime `Data` graph.
//!
//! Parsing maps a json document straight onto values candela already has: an
//! object becomes a map keyed by strings, an array becomes an array, and a
//! scalar becomes int/float/string/bool/null. No new runtime type is needed.
//!
//! Values are written into the pools with direct pushes rather than the
//! GC-aware `alloc_*` helpers. A json document is built in one uninterrupted
//! pass, and a partially built graph is not yet reachable from any register, so
//! running the collector mid-parse could reclaim it. Direct pushes never invoke
//! the collector; the slots they add are reclaimed by the next ordinary GC.

use crate::data::{Data, NULL};
use crate::rt::EnumType;
use crate::rt::Struct;
use crate::vm::{MapPool, ObjectPool, StringPool};
use std::collections::HashMap;

fn store_string(s: &str, str_pool: &mut StringPool) -> Data {
    if s.len() <= 6 {
        Data::small_str(s)
    } else {
        let id = str_pool.len() as u64;
        str_pool.push(s.to_owned());
        Data::large_str_id(id)
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl JsonParser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(
        &mut self,
        obj_pool: &mut ObjectPool,
        map_pool: &mut MapPool,
        str_pool: &mut StringPool,
    ) -> Result<Data, &'static str> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(obj_pool, map_pool, str_pool),
            Some(b'[') => self.parse_array(obj_pool, map_pool, str_pool),
            Some(b'"') => {
                let s = self.parse_string()?;
                Ok(store_string(&s, str_pool))
            }
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            Some(_) => Err("unexpected character"),
            None => Err("unexpected end of input"),
        }
    }

    fn parse_object(
        &mut self,
        obj_pool: &mut ObjectPool,
        map_pool: &mut MapPool,
        str_pool: &mut StringPool,
    ) -> Result<Data, &'static str> {
        self.pos += 1; // consume '{'
        let mut map: HashMap<Data, Data, _> = HashMap::default();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            let id = map_pool.len() as u32;
            map_pool.push(map);
            return Ok(Data::map(id));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err("object key must be a string");
            }
            let key = self.parse_string()?;
            let key_data = store_string(&key, str_pool);
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err("expected ':' after object key");
            }
            self.pos += 1;
            let val = self.parse_value(obj_pool, map_pool, str_pool)?;
            map.insert(key_data, val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err("expected ',' or '}' in object"),
            }
        }
        let id = map_pool.len() as u32;
        map_pool.push(map);
        Ok(Data::map(id))
    }

    fn parse_array(
        &mut self,
        obj_pool: &mut ObjectPool,
        map_pool: &mut MapPool,
        str_pool: &mut StringPool,
    ) -> Result<Data, &'static str> {
        self.pos += 1; // consume '['
        let mut elems: Vec<Data> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            let id = obj_pool.len() as u32;
            obj_pool.push(elems);
            return Ok(Data::array(id));
        }
        loop {
            let val = self.parse_value(obj_pool, map_pool, str_pool)?;
            elems.push(val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err("expected ',' or ']' in array"),
            }
        }
        let id = obj_pool.len() as u32;
        obj_pool.push(elems);
        Ok(Data::array(id))
    }

    fn parse_string(&mut self) -> Result<String, &'static str> {
        self.pos += 1; // consume opening quote
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err("unterminated string");
            };
            self.pos += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(esc) = self.peek() else {
                        return Err("unterminated escape");
                    };
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let cp = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&cp) {
                                // High surrogate: a low surrogate must follow.
                                if self.peek() != Some(b'\\') {
                                    return Err("unpaired surrogate");
                                }
                                self.pos += 1;
                                if self.peek() != Some(b'u') {
                                    return Err("unpaired surrogate");
                                }
                                self.pos += 1;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("invalid low surrogate");
                                }
                                let c = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                                match char::from_u32(c) {
                                    Some(ch) => out.push(ch),
                                    None => return Err("invalid unicode escape"),
                                }
                            } else {
                                match char::from_u32(cp) {
                                    Some(ch) => out.push(ch),
                                    None => return Err("invalid unicode escape"),
                                }
                            }
                        }
                        _ => return Err("invalid escape"),
                    }
                }
                // A UTF-8 continuation/lead byte: copy the raw byte through. The
                // input was a valid `&str`, so byte-wise copying preserves it.
                _ => out.push(c as char),
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, &'static str> {
        if self.pos + 4 > self.bytes.len() {
            return Err("truncated unicode escape");
        }
        let mut v: u32 = 0;
        for _ in 0..4 {
            let d = self.bytes[self.pos];
            self.pos += 1;
            let n = match d {
                b'0'..=b'9' => (d - b'0') as u32,
                b'a'..=b'f' => (d - b'a' + 10) as u32,
                b'A'..=b'F' => (d - b'A' + 10) as u32,
                _ => return Err("invalid hex digit"),
            };
            v = (v << 4) | n;
        }
        Ok(v)
    }

    fn parse_bool(&mut self) -> Result<Data, &'static str> {
        if self.bytes[self.pos..].starts_with(b"true") {
            self.pos += 4;
            Ok(Data::bool(true))
        } else if self.bytes[self.pos..].starts_with(b"false") {
            self.pos += 5;
            Ok(Data::bool(false))
        } else {
            Err("invalid literal")
        }
    }

    fn parse_null(&mut self) -> Result<Data, &'static str> {
        if self.bytes[self.pos..].starts_with(b"null") {
            self.pos += 4;
            Ok(NULL)
        } else {
            Err("invalid literal")
        }
    }

    fn parse_number(&mut self) -> Result<Data, &'static str> {
        let start = self.pos;
        let mut is_float = false;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            match c {
                b'0'..=b'9' => self.pos += 1,
                b'.' | b'e' | b'E' | b'+' | b'-' => {
                    is_float = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let text = match std::str::from_utf8(&self.bytes[start..self.pos]) {
            Ok(t) => t,
            Err(_) => return Err("invalid number"),
        };
        if !is_float
            && let Ok(n) = text.parse::<i32>()
        {
            return Ok(Data::int(n));
        }
        match text.parse::<f64>() {
            Ok(f) => Ok(Data::float(f)),
            Err(_) => Err("invalid number"),
        }
    }
}

/// Parses a json document into a `Data` graph. A malformed document returns a
/// short static reason.
pub fn json_parse(
    input: &str,
    obj_pool: &mut ObjectPool,
    map_pool: &mut MapPool,
    str_pool: &mut StringPool,
) -> Result<Data, &'static str> {
    let mut p = JsonParser {
        bytes: input.as_bytes(),
        pos: 0,
    };
    let v = p.parse_value(obj_pool, map_pool, str_pool)?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err("trailing characters after value");
    }
    Ok(v)
}

fn escape_into(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_json(
    d: Data,
    out: &mut String,
    obj_pool: &ObjectPool,
    map_pool: &MapPool,
    str_pool: &StringPool,
    structs: &[Struct],
    enums: &[EnumType],
) {
    if d.is_int() {
        out.push_str(&d.as_int().to_string());
    } else if d.is_float() {
        let f = d.as_float();
        if f.is_finite() {
            // Debug formatting keeps a decimal point (`1.0`, not `1`), so a
            // float round-trips back to a float rather than an int.
            out.push_str(&format!("{f:?}"));
        } else {
            out.push_str("null");
        }
    } else if d.is_bool() {
        out.push_str(if d.as_bool() { "true" } else { "false" });
    } else if d.is_null() {
        out.push_str("null");
    } else if d.is_string() {
        escape_into(d.as_str(str_pool), out);
    } else if d.is_array() {
        out.push('[');
        for (i, e) in obj_pool[d.as_array()].iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            write_json(*e, out, obj_pool, map_pool, str_pool, structs, enums);
        }
        out.push(']');
    } else if d.is_map() {
        out.push('{');
        for (i, (k, v)) in map_pool[d.as_map()].iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            // json object keys are strings. A string key is emitted directly; a
            // non-string key is rendered to its text form and quoted.
            if k.is_string() {
                escape_into(k.as_str(str_pool), out);
            } else {
                let key = k.format(obj_pool, str_pool, map_pool, structs, enums, true);
                escape_into(&key, out);
            }
            out.push(':');
            write_json(*v, out, obj_pool, map_pool, str_pool, structs, enums);
        }
        out.push('}');
    } else {
        // struct/enum have no json shape; emit their text form as a string.
        let s = d.format(obj_pool, str_pool, map_pool, structs, enums, true);
        escape_into(&s, out);
    }
}

/// Serializes a `Data` value to a json string.
pub fn json_stringify(
    d: Data,
    obj_pool: &ObjectPool,
    map_pool: &MapPool,
    str_pool: &StringPool,
    structs: &[Struct],
    enums: &[EnumType],
) -> String {
    let mut out = String::new();
    write_json(d, &mut out, obj_pool, map_pool, str_pool, structs, enums);
    out
}
