//! Byte-offset <-> LSP `Position` conversion.
//!
//! candela's spans (`candela::compiler::expr::Span`) are byte ranges into the
//! source text. LSP positions are (line, UTF-16 code unit) pairs. These are
//! small, allocation-free O(n) scans rather than a cached line-start table:
//! candela scripts are short, and recomputing on every request keeps this
//! module trivially correct instead of needing cache invalidation on edits.

use tower_lsp::lsp_types::Position;

/// Converts a byte offset into `text` to an LSP `Position`. Offsets past the
/// end of the text clamp to the last valid position.
#[must_use]
pub fn offset_to_position(text: &str, offset: u32) -> Position {
    let offset = (offset as usize).min(text.len());
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if i >= offset {
            break;
        }
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let character = text[line_start..offset].encode_utf16().count() as u32;
    Position::new(line, character)
}

/// Converts an LSP `Position` back to a byte offset into `text`. A position
/// past the end of its line clamps to the line's end; a line number past the
/// end of the text clamps to `text.len()`.
#[must_use]
pub fn position_to_offset(text: &str, pos: Position) -> u32 {
    let mut line = 0u32;
    let mut line_start = 0usize;
    if pos.line > 0 {
        let mut found = false;
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line += 1;
                if line == pos.line {
                    line_start = i + 1;
                    found = true;
                    break;
                }
            }
        }
        if !found {
            // Requested line is beyond the text: clamp to end.
            return text.len() as u32;
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);

    let mut utf16_count = 0u32;
    for (byte_idx, ch) in text[line_start..line_end].char_indices() {
        if utf16_count >= pos.character {
            return (line_start + byte_idx) as u32;
        }
        utf16_count += ch.len_utf16() as u32;
    }
    line_end as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_position_roundtrip_ascii() {
        let text = "fn main() {\n    print(1);\n}\n";
        let offset = text.find("print").unwrap() as u32;
        let pos = offset_to_position(text, offset);
        assert_eq!(pos, Position::new(1, 4));
        assert_eq!(position_to_offset(text, pos), offset);
    }

    #[test]
    fn offset_position_multibyte() {
        // "let s = \"caf\u{e9}\";": the e-acute is 2 bytes in UTF-8, 1 UTF-16 unit.
        let text = "let s = \"caf\u{e9}\";";
        let e_acute_byte_offset = text.find('\u{e9}').unwrap() as u32;
        let pos = offset_to_position(text, e_acute_byte_offset);
        // Everything before it is ASCII, so UTF-16 column == byte column.
        assert_eq!(pos.character, e_acute_byte_offset);
        assert_eq!(position_to_offset(text, pos), e_acute_byte_offset);
    }

    #[test]
    fn clamps_past_end() {
        let text = "fn main() {}\n";
        assert_eq!(
            offset_to_position(text, 9999),
            offset_to_position(text, text.len() as u32)
        );
        assert_eq!(
            position_to_offset(text, Position::new(50, 0)),
            text.len() as u32
        );
    }
}
