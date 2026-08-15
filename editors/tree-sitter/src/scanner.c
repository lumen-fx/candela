#include "tree_sitter/parser.h"

// The raw region of a macro invocation, `name!( ... )`. The region ends at the
// parenthesis that balances the one that opened it; a parenthesis inside a
// string literal or after `//` on a line does not count. That is the scan
// `region_len` in src/macros.rs performs, and the token this yields covers the
// same text the expander receives.

enum TokenType {
  MACRO_BODY,
  ERROR_SENTINEL,
};

void *tree_sitter_candela_external_scanner_create(void) { return NULL; }

void tree_sitter_candela_external_scanner_destroy(void *payload) { (void)payload; }

unsigned tree_sitter_candela_external_scanner_serialize(void *payload, char *buffer) {
  (void)payload;
  (void)buffer;
  return 0;
}

void tree_sitter_candela_external_scanner_deserialize(void *payload, const char *buffer,
                                                     unsigned length) {
  (void)payload;
  (void)buffer;
  (void)length;
}

/// Consumes the string literal that opens at the current position, including
/// both quotes. False when the file ends before the closing quote.
static bool scan_string(TSLexer *lexer) {
  lexer->advance(lexer, false);
  while (!lexer->eof(lexer)) {
    if (lexer->lookahead == '\\') {
      lexer->advance(lexer, false);
      if (lexer->eof(lexer)) {
        return false;
      }
      lexer->advance(lexer, false);
      continue;
    }
    if (lexer->lookahead == '"') {
      lexer->advance(lexer, false);
      return true;
    }
    lexer->advance(lexer, false);
  }
  return false;
}

/// Consumes the rest of a `//` comment, stopping at the line ending, which the
/// caller reads as ordinary text.
static void scan_line_comment(TSLexer *lexer) {
  while (!lexer->eof(lexer) && lexer->lookahead != '\n' && lexer->lookahead != '\r') {
    lexer->advance(lexer, false);
  }
}

/// Consumes the region up to the parenthesis that closes it, which is left for
/// the parser. False for an empty region and for one the file ends inside.
static bool scan_region(TSLexer *lexer) {
  unsigned depth = 1;
  bool consumed = false;
  while (!lexer->eof(lexer)) {
    switch (lexer->lookahead) {
      case ')':
        depth--;
        if (depth == 0) {
          return consumed;
        }
        lexer->advance(lexer, false);
        break;
      case '(':
        depth++;
        lexer->advance(lexer, false);
        break;
      case '"':
        if (!scan_string(lexer)) {
          return false;
        }
        break;
      case '/':
        lexer->advance(lexer, false);
        if (lexer->lookahead == '/') {
          scan_line_comment(lexer);
        }
        break;
      default:
        lexer->advance(lexer, false);
        break;
    }
    consumed = true;
  }
  return false;
}

bool tree_sitter_candela_external_scanner_scan(void *payload, TSLexer *lexer,
                                               const bool *valid_symbols) {
  (void)payload;
  // Every token is valid while the parser recovers from an error, and a region
  // scanned there would swallow the rest of the file.
  if (valid_symbols[ERROR_SENTINEL] || !valid_symbols[MACRO_BODY]) {
    return false;
  }
  if (!scan_region(lexer)) {
    return false;
  }
  lexer->result_symbol = MACRO_BODY;
  return true;
}
