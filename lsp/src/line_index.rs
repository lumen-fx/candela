//! Byte-offset <-> LSP `Position` conversion.
//!
//! candela's spans (`candela::compiler::expr::Span`) are byte ranges into the
//! source text. LSP positions are (line, UTF-16 code unit) pairs. A
//! `LineIndex` records where each line starts once, so a feature that places
//! many spans in one file (the outline, every reference, every token) pays
//! for one scan of the text rather than one per span.

use tower_lsp::lsp_types::{Position, Range};

/// The line starts of one text.
pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Self { text, starts }
    }

    /// The `Position` of a byte offset. An offset past the end of the text
    /// clamps to the end; one inside a character, to that character's start.
    #[must_use]
    pub fn position(&self, offset: usize) -> Position {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let start = self.starts[line];
        let character = self.text[start..offset].encode_utf16().count();
        Position::new(
            u32::try_from(line).unwrap_or(u32::MAX),
            u32::try_from(character).unwrap_or(u32::MAX),
        )
    }

    /// The `Range` of a byte range.
    #[must_use]
    pub fn range(&self, start: usize, end: usize) -> Range {
        Range::new(self.position(start), self.position(end))
    }

    /// The byte offset of a `Position`. A position past the end of its line
    /// clamps to the line's end; a line past the end of the text, to the
    /// text's end.
    #[must_use]
    pub fn offset(&self, pos: Position) -> usize {
        let Some(&start) = self.starts.get(pos.line as usize) else {
            return self.text.len();
        };
        let end = self
            .starts
            .get(pos.line as usize + 1)
            .map_or(self.text.len(), |&next| next - 1);
        let mut units = 0u32;
        for (i, ch) in self.text[start..end].char_indices() {
            if units >= pos.character {
                return start + i;
            }
            units += u32::try_from(ch.len_utf16()).unwrap_or(2);
        }
        end
    }
}

/// The byte offset of a `Position` in `text`.
#[must_use]
pub fn position_to_offset(text: &str, pos: Position) -> usize {
    LineIndex::new(text).offset(pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_position_roundtrip_ascii() {
        let text = "fn main() {\n    print(1);\n}\n";
        let index = LineIndex::new(text);
        let offset = text.find("print").unwrap();
        let pos = index.position(offset);
        assert_eq!(pos, Position::new(1, 4));
        assert_eq!(index.offset(pos), offset);
    }

    #[test]
    fn offset_position_multibyte() {
        // "let s = \"caf\u{e9}\";": the e-acute is 2 bytes in UTF-8, 1 UTF-16 unit.
        let text = "let s = \"caf\u{e9}\"; x";
        let index = LineIndex::new(text);
        let e_acute = text.find('\u{e9}').unwrap();
        let pos = index.position(e_acute);
        // Everything before it is ASCII, so UTF-16 column == byte column.
        assert_eq!(pos.character as usize, e_acute);
        assert_eq!(index.offset(pos), e_acute);
        // Past it, a column counts the character once.
        let x = text.find('x').unwrap();
        assert_eq!(index.position(x).character as usize, x - 1);
        assert_eq!(index.offset(index.position(x)), x);
        // Inside it, the character's start.
        assert_eq!(index.position(e_acute + 1), pos);
    }

    #[test]
    fn clamps_past_end() {
        let text = "fn main() {}\n";
        let index = LineIndex::new(text);
        assert_eq!(index.position(9999), index.position(text.len()));
        assert_eq!(index.offset(Position::new(50, 0)), text.len());
        assert_eq!(
            index.offset(Position::new(0, 500)),
            text.find('\n').unwrap()
        );
    }
}
