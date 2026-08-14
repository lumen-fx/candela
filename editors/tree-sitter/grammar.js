/**
 * @file Tree-sitter grammar for the candela language (.cdl)
 * @license Apache-2.0
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

// Binary operator levels, loosest first, matching `check_op` in
// src/parser/parser_expr.rs and the table in docs/docs/reference/operators.md.
// The prefix operators bind tighter than `^` and looser than the postfix forms.
const PREC = {
  or: 1,
  and: 2,
  equality: 3,
  comparison: 4,
  additive: 5,
  multiplicative: 6,
  power: 7,
  unary: 8,
  postfix: 9,
  // A `name { ... }` struct literal competes with the block that follows a
  // condition, so it is resolved dynamically; see the `conflicts` list.
  struct_literal: 10,
};

/**
 * One or more `rule`, separated by `separator`.
 * @param {string} separator
 * @param {RuleOrLiteral} rule
 */
function sepBy1(separator, rule) {
  return seq(rule, repeat(seq(separator, rule)));
}

/**
 * Zero or more `rule`, separated by `separator`, with an optional trailing one.
 * @param {string} separator
 * @param {RuleOrLiteral} rule
 */
function sepByTrailing(separator, rule) {
  return optional(seq(sepBy1(separator, rule), optional(separator)));
}

module.exports = grammar({
  name: 'candela',

  extras: ($) => [/\s/, $.comment],

  word: ($) => $.identifier,

  conflicts: ($) => [
    // `while x { ... }`: the parser cannot know whether `x` names a struct
    // being built or is the whole condition until it has read the brace body.
    [$._expression, $.struct_literal],
    // `{` opens a block statement and also a map literal.
    [$.block, $.map_literal],
  ],

  rules: {
    source_file: ($) => repeat($._declaration),

    // Only these appear at the top level of a file; `parse_file` in
    // src/parser/parser.rs rejects everything else, including `let`.
    _declaration: ($) =>
      choice(
        $.import_declaration,
        $.function_declaration,
        $.struct_declaration,
        $.enum_declaration,
        $.impl_block,
        $.dylib_block,
        $.host_block,
      ),

    // ------------------------------------------------------------------
    // Declarations
    // ------------------------------------------------------------------

    import_declaration: ($) =>
      seq(
        'import',
        field('path', $.string_literal),
        optional(seq('as', field('alias', $.identifier))),
        ';',
      ),

    function_declaration: ($) =>
      seq(
        'fn',
        field('name', $.identifier),
        field('parameters', $.parameter_list),
        optional(field('return_type', $.return_type)),
        field('body', $.block),
      ),

    parameter_list: ($) => seq('(', sepByTrailing(',', $.parameter), ')'),

    parameter: ($) =>
      seq(
        field('name', $.identifier),
        optional(seq(':', field('type', $._type))),
      ),

    return_type: ($) => seq('->', $._type),

    struct_declaration: ($) =>
      seq('struct', field('name', $.identifier), field('body', $.field_declaration_list)),

    field_declaration_list: ($) =>
      seq('{', sepBy1(',', $.field_declaration), optional(','), '}'),

    field_declaration: ($) =>
      seq(field('name', $.identifier), ':', field('type', $._type)),

    enum_declaration: ($) =>
      seq('enum', field('name', $.identifier), field('body', $.enum_variant_list)),

    enum_variant_list: ($) => seq('{', sepByTrailing(',', $.enum_variant), '}'),

    enum_variant: ($) =>
      seq(field('name', $.identifier), optional(field('payload', $.payload_type_list))),

    payload_type_list: ($) => seq('(', sepByTrailing(',', $._type), ')'),

    // Methods lower to mangled free functions, so an impl block holds nothing
    // but `fn` declarations.
    impl_block: ($) =>
      seq('impl', field('type', $._type_identifier), '{', repeat($.function_declaration), '}'),

    dylib_block: ($) =>
      seq('dylib', field('path', $.string_literal), field('body', $.signature_list)),

    host_block: ($) =>
      seq('host', field('namespace', $.string_literal), field('body', $.signature_list)),

    signature_list: ($) => seq('{', repeat($.function_signature), '}'),

    // `int add(int, int);`, or `log(string);` for a function returning null.
    // Parameters are types only; `...` marks a variadic host function.
    function_signature: ($) =>
      seq(
        optional(field('return_type', $._type)),
        field('name', $.identifier),
        '(',
        optional(choice($.variadic_parameter, sepByTrailing(',', $._type))),
        ')',
        ';',
      ),

    variadic_parameter: (_) => '...',

    // ------------------------------------------------------------------
    // Types
    // ------------------------------------------------------------------

    _type: ($) => choice($._atomic_type, $.union_type),

    _atomic_type: ($) =>
      choice($._type_identifier, $.qualified_type, $.array_type, $.map_type),

    union_type: ($) => seq($._atomic_type, repeat1(seq('|', $._atomic_type))),

    array_type: ($) => seq(field('element', $._atomic_type), '[', ']'),

    map_type: ($) => seq('{', field('key', $._type), ':', field('value', $._type), '}'),

    // `a::b::c` nests to the left, so the last segment is the name and
    // everything before it is the module path.
    qualified_type: ($) =>
      seq(
        field('module', choice($._type_identifier, $.qualified_type)),
        '::',
        field('name', $._type_identifier),
      ),

    _type_identifier: ($) => alias($.identifier, $.type_identifier),

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    block: ($) => seq('{', repeat($._statement), optional($._expression), '}'),

    _statement: ($) =>
      choice(
        $.let_declaration,
        $.assignment_statement,
        $.expression_statement,
        $.return_statement,
        $.break_statement,
        $.continue_statement,
        $.while_statement,
        $.for_statement,
        $.loop_statement,
        $.match_statement,
        $.try_statement,
        $.block,
        $.struct_declaration,
        $.enum_declaration,
      ),

    // A block-bodied expression stands alone as a statement; anything else
    // needs the semicolon.
    expression_statement: ($) => choice(seq($._expression, ';'), prec(1, $.if_expression)),

    let_declaration: ($) =>
      seq('let', field('name', $.identifier), '=', field('value', $._expression), ';'),

    assignment_statement: ($) =>
      seq(
        field('left', $._expression),
        field('operator', choice('=', '+=', '-=', '*=', '/=', '%=', '^=')),
        field('right', $._expression),
        ';',
      ),

    return_statement: ($) => seq('return', optional(field('value', $._expression)), ';'),

    break_statement: (_) => seq('break', ';'),

    continue_statement: (_) => seq('continue', ';'),

    while_statement: ($) =>
      seq('while', field('condition', $._expression), field('body', $.block)),

    // `for x in list`, `for i in low..high`, and `for i in ..high`, which
    // counts from zero.
    for_statement: ($) =>
      seq(
        'for',
        field('binding', $.identifier),
        'in',
        field('iterable', choice($.range_expression, $._expression)),
        field('body', $.block),
      ),

    range_expression: ($) =>
      seq(optional(field('start', $._expression)), '..', field('end', $._expression)),

    loop_statement: ($) => seq('loop', field('body', $.block)),

    match_statement: ($) =>
      seq('match', field('value', $._expression), field('body', $.match_arm_list)),

    match_arm_list: ($) => seq('{', repeat($.match_arm), '}'),

    // Arms are not comma-separated. A pattern is an expression: an enum
    // variant with bindings, a qualified name, or a literal compared for
    // equality. `_` is the wildcard, and it comes last.
    match_arm: ($) =>
      seq(field('pattern', $._expression), '=>', field('body', $.block)),

    try_statement: ($) => seq('try', field('body', $.block), repeat1($.catch_clause)),

    // `catch "kind" { ... }` handles one error kind; `catch e { ... }` binds
    // every remaining kind to a variable.
    catch_clause: ($) =>
      seq(
        'catch',
        field('pattern', choice($.string_literal, $.identifier)),
        field('body', $.block),
      ),

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    _expression: ($) =>
      choice(
        $.identifier,
        $.qualified_identifier,
        $.integer_literal,
        $.float_literal,
        $.string_literal,
        $.boolean_literal,
        $.null_literal,
        $.array_literal,
        $.map_literal,
        $.struct_literal,
        $.call_expression,
        $.method_call_expression,
        $.field_expression,
        $.index_expression,
        $.slice_expression,
        $.unary_expression,
        $.binary_expression,
        $.parenthesized_expression,
        $.closure_expression,
        $.if_expression,
      ),

    parenthesized_expression: ($) => seq('(', $._expression, ')'),

    unary_expression: ($) =>
      prec.right(PREC.unary, seq(field('operator', choice('-', '!')), field('operand', $._expression))),

    binary_expression: ($) => {
      const table = [
        [PREC.or, '||'],
        [PREC.and, '&&'],
        [PREC.equality, '=='],
        [PREC.equality, '!='],
        [PREC.comparison, '<'],
        [PREC.comparison, '<='],
        [PREC.comparison, '>'],
        [PREC.comparison, '>='],
        [PREC.additive, '+'],
        [PREC.additive, '-'],
        [PREC.multiplicative, '*'],
        [PREC.multiplicative, '/'],
        [PREC.multiplicative, '%'],
      ];

      return choice(
        ...table.map(([precedence, operator]) =>
          prec.left(
            Number(precedence),
            seq(
              field('left', $._expression),
              field('operator', operator),
              field('right', $._expression),
            ),
          ),
        ),
        prec.right(
          PREC.power,
          seq(field('left', $._expression), field('operator', '^'), field('right', $._expression)),
        ),
      );
    },

    // A call names a function directly: candela has no call on an arbitrary
    // expression. A namespaced name is either an imported module's function
    // or an enum variant with a payload.
    call_expression: ($) =>
      prec(
        PREC.postfix,
        seq(
          field('function', choice($.identifier, $.qualified_identifier)),
          field('arguments', $.arguments),
        ),
      ),

    method_call_expression: ($) =>
      prec(
        PREC.postfix,
        seq(
          field('receiver', $._expression),
          '.',
          field('method', choice($.identifier, $.qualified_identifier)),
          field('arguments', $.arguments),
        ),
      ),

    field_expression: ($) =>
      prec(PREC.postfix, seq(field('object', $._expression), '.', field('field', $.identifier))),

    index_expression: ($) =>
      prec(PREC.postfix, seq(field('object', $._expression), '[', field('index', $._expression), ']')),

    // `a[low..high]` and `a[..high]`. There is no open-ended `a[low..]`.
    slice_expression: ($) =>
      prec(
        PREC.postfix,
        seq(
          field('object', $._expression),
          '[',
          optional(field('start', $._expression)),
          '..',
          field('end', $._expression),
          ']',
        ),
      ),

    arguments: ($) => seq('(', sepByTrailing(',', $._expression), ')'),

    array_literal: ($) => seq('[', sepByTrailing(',', $._expression), ']'),

    // Map literals take no trailing comma; struct literals do.
    map_literal: ($) => seq('{', optional(sepBy1(',', $.map_entry)), '}'),

    map_entry: ($) => seq(field('key', $._expression), ':', field('value', $._expression)),

    struct_literal: ($) =>
      prec.dynamic(
        PREC.struct_literal,
        seq(
          field('name', choice($.identifier, $.qualified_identifier)),
          field('body', $.field_initializer_list),
        ),
      ),

    field_initializer_list: ($) =>
      seq('{', sepBy1(',', $.field_initializer), optional(','), '}'),

    field_initializer: ($) =>
      seq(field('name', $.identifier), ':', field('value', $._expression)),

    // An anonymous function. It takes no return annotation and captures
    // nothing from the surrounding scope.
    closure_expression: ($) =>
      seq('fn', field('parameters', $.parameter_list), field('body', $.block)),

    // Every branch yields a value, so the `else` is required.
    if_expression: ($) =>
      prec.right(
        seq(
          'if',
          field('condition', $._expression),
          field('consequence', $.block),
          repeat($.else_if_clause),
          optional($.else_clause),
        ),
      ),

    else_if_clause: ($) =>
      seq('else', 'if', field('condition', $._expression), field('consequence', $.block)),

    else_clause: ($) => seq('else', field('body', $.block)),

    qualified_identifier: ($) =>
      seq(
        field('module', choice($.identifier, $.qualified_identifier)),
        '::',
        field('name', $.identifier),
      ),

    // ------------------------------------------------------------------
    // Tokens
    // ------------------------------------------------------------------

    identifier: (_) => /[a-zA-Z_][a-zA-Z0-9_]*/,

    integer_literal: (_) => /[0-9]+/,

    // A leading digit is optional, so `.5` is a float; a trailing point is not.
    float_literal: (_) => /[0-9]*\.[0-9]+/,

    string_literal: ($) =>
      seq('"', repeat(choice($.escape_sequence, token.immediate(/[^"\\]+/))), '"'),

    // `\n`, `\t`, `\r`, `\\`, `\"`, and `\0` are translated; any other
    // backslash pair is kept as written.
    escape_sequence: (_) => token.immediate(/\\./),

    boolean_literal: (_) => choice('true', 'false'),

    null_literal: (_) => 'null',

    comment: (_) => token(seq('//', /[^\r\n]*/)),
  },
});
