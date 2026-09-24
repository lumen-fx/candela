//! json parse and stringify over the runtime `Data` graph.
//!
//! Parsing maps a json document straight onto values candela already has: an
//! object becomes a map keyed by strings, an array becomes an array, and a
//! scalar becomes int/float/string/bool/null. No new runtime type is needed.
//!
//! Parsed values allocate the way the program's own do, reusing freed slots,
//! so a parse can start or carry on a collection. The value is built
//! top-down to stay reachable while it does: the outermost list or map sits on
//! the recursion stack, a collection root, until the parse ends, and each
//! entry goes into its parent before anything inside it is allocated. A
//! string is interned, since a map finds a string key by its slot.

use crate::data::{Data, NULL};
use crate::gc::GcState;
use crate::gc::Heap;
use crate::rt::EnumType;
use crate::rt::Struct;
use crate::vm::map_insert;
use crate::vm::{MapPool, ObjectPool, RegisterFile, StringPool};

/// How deeply objects and arrays may nest. Values are read by recursive
/// descent, so without a ceiling a long run of `[` exhausts the native stack
/// and kills the process instead of raising. Real documents stay far below
/// this.
const MAX_NESTING_DEPTH: u32 = 128;

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
    depth: u32,
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

    /// The next value: a scalar, a string, or an empty list or map whose
    /// opening bracket is consumed, for [`Self::fill`] to fill in once the
    /// caller has made it reachable.
    fn parse_value(&mut self, heap: &mut Heap<'_>) -> Result<Data, &'static str> {
        self.skip_ws();
        match self.peek() {
            Some(open @ (b'{' | b'[')) => {
                self.depth += 1;
                if self.depth > MAX_NESTING_DEPTH {
                    return Err("nesting too deep");
                }
                self.pos += 1;
                Ok(if open == b'{' {
                    heap.map()
                } else {
                    heap.array()
                })
            }
            Some(b'"') => {
                let s = self.parse_string()?;
                Ok(heap.string(&s))
            }
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            Some(_) => Err("unexpected character"),
            None => Err("unexpected end of input"),
        }
    }

    /// Reads the entries of `d`, a value [`Self::parse_value`] answered, up
    /// to its closing bracket. Nothing to do for a scalar or a string.
    fn fill(&mut self, heap: &mut Heap<'_>, d: Data) -> Result<(), &'static str> {
        if d.is_array() {
            self.fill_array(heap, d.as_array())?;
        } else if d.is_map() {
            self.fill_object(heap, d.as_map())?;
        } else {
            return Ok(());
        }
        self.depth -= 1;
        Ok(())
    }

    fn fill_object(&mut self, heap: &mut Heap<'_>, id: usize) -> Result<(), &'static str> {
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(());
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err("object key must be a string");
            }
            let key = self.parse_string()?;
            let key = heap.string(&key);
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err("expected ':' after object key");
            }
            self.pos += 1;
            // The key goes in first, so the value's allocation cannot free it.
            // A key the object already had takes the later value, in the place
            // the first one had.
            map_insert(&mut heap.maps[id], key, NULL, heap.strings);
            let val = self.parse_value(heap)?;
            map_insert(&mut heap.maps[id], key, val, heap.strings);
            self.fill(heap, val)?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err("expected ',' or '}' in object"),
            }
        }
    }

    fn fill_array(&mut self, heap: &mut Heap<'_>, id: usize) -> Result<(), &'static str> {
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(());
        }
        loop {
            let val = self.parse_value(heap)?;
            heap.objs[id].push(val);
            self.fill(heap, val)?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err("expected ',' or ']' in array"),
            }
        }
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
                c if c.is_ascii() => out.push(c as char),
                // The lead byte of a multi-byte character: the input is a valid
                // `&str`, so the whole character is copied through as text.
                c => {
                    let start = self.pos - 1;
                    let len = if c >= 0xF0 {
                        4
                    } else if c >= 0xE0 {
                        3
                    } else {
                        2
                    };
                    let end = (start + len).min(self.bytes.len());
                    match std::str::from_utf8(&self.bytes[start..end]) {
                        Ok(ch) => out.push_str(ch),
                        Err(_) => return Err("invalid utf-8"),
                    }
                    self.pos = end;
                }
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
        if !is_float && let Ok(n) = text.parse::<i64>() {
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
///
/// A collection the parse starts reads `registers` and `recursion_stack` as
/// its roots, so every other value the program still holds has to be
/// reachable from them, and the caller stores the result before it allocates
/// again.
pub fn json_parse(
    input: &str,
    obj_pool: &mut ObjectPool,
    map_pool: &mut MapPool,
    str_pool: &mut StringPool,
    registers: &RegisterFile,
    recursion_stack: &mut RegisterFile,
    gc: &mut GcState,
) -> Result<Data, &'static str> {
    let mut heap = Heap {
        objs: obj_pool,
        maps: map_pool,
        strings: str_pool,
        gc,
        registers,
        recursion_stack,
    };
    let mut p = JsonParser {
        bytes: input.as_bytes(),
        pos: 0,
        depth: 0,
    };
    let v = p.parse_value(&mut heap)?;
    heap.recursion_stack.0.push(v);
    let filled = p.fill(&mut heap, v);
    heap.recursion_stack.0.pop();
    filled?;
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

#[cfg(test)]
mod json_tests {
    use super::MAX_NESTING_DEPTH;
    use crate::data::Data;
    use crate::gc::GcState;
    use crate::vm::{MapPool, ObjectPool, Pool, RegisterFile, StringPool};

    fn pools() -> (ObjectPool, MapPool, StringPool) {
        (Pool(Vec::new()), Pool(Vec::new()), StringPool::default())
    }

    /// Parses `input` with no roots beside the parse itself.
    fn json_parse(
        input: &str,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
    ) -> Result<Data, &'static str> {
        super::json_parse(
            input,
            objs,
            maps,
            strings,
            &RegisterFile(Vec::new()),
            &mut RegisterFile(Vec::new()),
            &mut GcState::default(),
        )
    }

    #[test]
    fn malformed_input_is_rejected() {
        for bad in [
            "nope",
            "[1,",
            "{oops",
            "",
            "{\"a\":}",
            "[1] extra",
            "\"open",
        ] {
            let (mut obj, mut map, mut strings) = pools();
            assert!(
                json_parse(bad, &mut obj, &mut map, &mut strings).is_err(),
                "{bad:?} must not parse"
            );
        }
    }

    /// Values are read by recursive descent, so a long run of openers used to
    /// exhaust the native stack and kill the process instead of raising.
    #[test]
    fn nesting_past_the_limit_raises_instead_of_overflowing() {
        let (mut obj, mut map, mut strings) = pools();
        assert_eq!(
            json_parse(&"[".repeat(200_000), &mut obj, &mut map, &mut strings),
            Err("nesting too deep")
        );

        let (mut obj, mut map, mut strings) = pools();
        assert_eq!(
            json_parse(&"{\"a\":".repeat(200_000), &mut obj, &mut map, &mut strings),
            Err("nesting too deep")
        );
    }

    #[test]
    fn nesting_up_to_the_limit_parses() {
        let depth = MAX_NESTING_DEPTH as usize;
        let doc = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let (mut obj, mut map, mut strings) = pools();
        assert!(json_parse(&doc, &mut obj, &mut map, &mut strings).is_ok());
    }

    /// A string over six bytes is boxed as its pool slot, and the same text
    /// can sit in more than one slot. A parsed key is found by its text, the
    /// way a program looks it up, whatever slot the program's string is in.
    #[test]
    fn object_keys_are_found_by_their_text() {
        let (mut obj, mut map, mut strings) = pools();
        let parsed = json_parse(
            "{\"n\": 7, \"long_key_name\": 42}",
            &mut obj,
            &mut map,
            &mut strings,
        )
        .expect("valid object parses");
        assert!(parsed.is_map());

        let short = Data::p_str("n", &mut strings);
        let long = Data::p_str("long_key_name", &mut strings);
        strings.push(String::from("long_key_name"));
        let other_slot = Data::large_str_id((strings.len() - 1) as u64);
        let entries = &map[parsed.as_map()];
        let get = |key: Data| {
            entries
                .get(&crate::vm::KeyByText {
                    key,
                    strings: &strings,
                })
                .copied()
                .map(Data::as_int)
        };
        assert_eq!(get(short), Some(7));
        assert_eq!(get(long), Some(42));
        assert_eq!(get(other_slot), Some(42));
    }

    /// A raw multi-byte character is text, not a run of Latin-1 bytes, so it
    /// reads back equal to the same string written in a program.
    #[test]
    fn raw_non_ascii_characters_parse_as_themselves() {
        for text in ["\u{e9}", "a\u{e9}b", "\u{4e2d}\u{6587} and \u{1f600} here"] {
            let (mut obj, mut map, mut strings) = pools();
            let doc = format!("\"{text}\"");
            let parsed =
                json_parse(&doc, &mut obj, &mut map, &mut strings).expect("valid string parses");
            assert_eq!(parsed, Data::p_str(text, &mut strings), "{text:?}");
        }
    }

    /// Equal strings anywhere in a document share one slot, so values compare
    /// equal as well as keys.
    #[test]
    fn equal_strings_share_a_pool_slot() {
        let (mut obj, mut map, mut strings) = pools();
        let parsed = json_parse(
            "[\"a_long_repeated_value\", \"a_long_repeated_value\"]",
            &mut obj,
            &mut map,
            &mut strings,
        )
        .expect("valid array parses");
        let items = &obj[parsed.as_array()];
        assert_eq!(items[0], items[1]);
    }

    /// A document with thousands of lists and maps starts a collection
    /// partway through its parse, which marks and sweeps before the parse
    /// ends, with a dropped earlier parse on the heap for the sweep to free and
    /// the parse to reuse. What the parse has built so far is reachable only
    /// through the recursion stack and its parents, and it all survives.
    #[test]
    fn a_collection_mid_parse_keeps_the_partial_value() {
        let entries: Vec<String> = (0..3000)
            .map(|i| {
                format!(
                    "{{\"id\":{i},\"name\":\"an entry named after {i}\",\"tags\":[{i},\"tag {i} of many\"]}}"
                )
            })
            .collect();
        let doc = format!("[{}]", entries.join(","));
        let (mut objs, mut maps, mut strings) = pools();
        let registers = RegisterFile(Vec::new());
        let mut stack = RegisterFile(Vec::new());
        let mut gc = GcState::default();
        let mut parse = |objs: &mut ObjectPool, maps: &mut MapPool, strings: &mut StringPool| {
            super::json_parse(&doc, objs, maps, strings, &registers, &mut stack, &mut gc)
                .expect("the document parses")
        };
        parse(&mut objs, &mut maps, &mut strings);
        let parsed = parse(&mut objs, &mut maps, &mut strings);
        assert!(gc.cycles() > 0, "the parses never started a collection");
        assert!(stack.0.is_empty(), "the parse left its root behind");
        let text = super::json_stringify(parsed, &objs, &maps, &strings, &[], &[]);
        assert!(
            text == doc,
            "the parsed value reads back as a different document"
        );
    }
}
