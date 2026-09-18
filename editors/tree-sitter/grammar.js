/**
 * @file Tree-sitter grammar for the candela language (.cdl)
 * @license Apache-2.0
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

// Binary operator levels, loosest first, matching `check_op` in
// src/parser/parser_expr.rs and the table in docs/docs/reference/operators.md.
// The prefix operators bind tighter than `^` and looser than the postfix forms.
//
// Everything a bare name can grow into sits at the postfix level: a call, a
// struct literal, a namespaced path, and the name on its own. Holding them
// level is what lets the parser carry all four readings of `a <` forward
// together, since a `<` opens a type argument list and is also the comparison
// operator; see the `conflicts` list.
const PREC = {
  or: 1,
  and: 2,
  equality: 3,
  comparison: 4,
  bit_or: 5,
  bit_xor: 6,
  bit_and: 7,
  shift: 8,
  additive: 9,
  multiplicative: 10,
  power: 11,
  unary: 12,
  postfix: 13,
  // A `name { ... }` struct literal competes with the block that follows a
  // condition, so it is resolved dynamically; see the `conflicts` list.
  struct_literal: 14,
  // `a < b > (c)` reads as a comparison and as a call naming a type argument,
  // and both readings run to the end of the expression. The compiler takes the
  // type argument, so the branch that reads one carries this dynamic
  // precedence and wins the tie.
  type_arguments: 15,
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

  // The region between a macro's parentheses ends at the parenthesis that
  // balances the one that opened it, which no token rule can describe. The
  // scanner in src/scanner.c finds that end the way `region_len` in
  // src/macros.rs does, and yields the region as one token. The sentinel after
  // it belongs to no rule; it is how the scanner sees that the parser is
  // recovering from an error, where every token is valid at once.
  externals: ($) => [$.macro_body, $._error_sentinel],

  conflicts: ($) => [
    // `while x { ... }`: the parser cannot know whether `x` names a struct
    // being built or is the whole condition until it has read the brace body.
    [$._expression, $.struct_literal],
    // `{` opens a block statement and also a map literal.
    [$.block, $.map_literal],
    // `a < b`, `a<b>(c)`, `a<b>{ ... }` and `a<b>::c` all start alike, and
    // only the token after the matching `>` tells them apart, so all four
    // readings are carried until it arrives.
    [$._expression, $.call_expression, $.struct_literal, $.qualified_identifier],
    // The same choice after a `.`, where the name is a field or the method of
    // a call that may name type arguments.
    [$.field_expression, $.method_call_expression, $.qualified_identifier],
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
        optional(field('type_parameters', $.type_parameters)),
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
      seq(
        'struct',
        field('name', $.identifier),
        optional(field('type_parameters', $.type_parameters)),
        field('body', $.field_declaration_list),
      ),

    field_declaration_list: ($) =>
      seq('{', sepBy1(',', $.field_declaration), optional(','), '}'),

    field_declaration: ($) =>
      seq(field('name', $.identifier), ':', field('type', $._type)),

    enum_declaration: ($) =>
      seq(
        'enum',
        field('name', $.identifier),
        optional(field('type_parameters', $.type_parameters)),
        field('body', $.enum_variant_list),
      ),

    enum_variant_list: ($) => seq('{', sepByTrailing(',', $.enum_variant), '}'),

    enum_variant: ($) =>
      seq(field('name', $.identifier), optional(field('payload', $.payload_type_list))),

    payload_type_list: ($) => seq('(', sepByTrailing(',', $._type), ')'),

    // Methods lower to mangled free functions, so an impl block holds nothing
    // but `fn` declarations.
    //
    // An `impl` on a generic type names type arguments in its header, either a
    // parameter it binds for the methods (`impl Cell<T>`) or the concrete
    // instantiation the block applies to (`impl Signal<int>`). Both are spelt
    // as a type argument list.
    impl_block: ($) =>
      seq(
        'impl',
        field('type', $._type_identifier),
        optional(field('type_arguments', $.type_arguments)),
        '{',
        repeat($.function_declaration),
        '}',
      ),

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
      choice(
        $._type_identifier,
        $.qualified_type,
        $.generic_type,
        $.array_type,
        $.map_type,
        $.function_type,
        $.parenthesized_type,
      ),

    // `fn(int, int) -> int`, the type of a position that holds a function. The
    // return type runs to the end of the type, so the `[]` in
    // `fn(int) -> int[]` belongs to it; `(fn(int) -> int)[]` is the list of
    // functions.
    function_type: ($) =>
      prec.right(
        seq(
          'fn',
          '(',
          optional(sepByTrailing(',', $._type)),
          ')',
          optional(seq('->', field('return_type', $._type))),
        ),
      ),

    parenthesized_type: ($) => seq('(', $._type, ')'),

    // `Cell<int>`, and nested as deep as it goes: a type argument is an
    // ordinary type, so `Cell<Cell<int>>` and `Cell<int>[]` both spell out.
    generic_type: ($) =>
      seq(
        field('name', choice($._type_identifier, $.qualified_type)),
        field('type_arguments', $.type_arguments),
      ),

    // The names a declaration binds: `struct Cell<T>`, `fn first<T>`. Each is
    // a bare name, never a composite type.
    type_parameters: ($) => seq('<', sepBy1(',', $._type_identifier), '>'),

    // The types a use names. A type argument list has no spelling of its own,
    // so in an expression one stands only where a `(`, a `{` or a `::` follows
    // the closing `>`, and here that falls out of the grammar, because nothing
    // else accepts a list: `a < b` and `a < b > c` stay the comparisons they
    // have always been.
    //
    // The compiler, in `type_args_ahead`, narrows it twice more, and this
    // grammar deliberately does not: in an `if`, `while` or `for` header a
    // struct literal cannot start, so a following `{` does not make a list
    // there, and after a `.` a following `::` does not either. Both accept
    // here and are refused by the compiler, which is the direction to err in;
    // the language server reports them while highlighting stays steady. See
    // the "When `<` is a comparison" section of
    // docs/docs/language/generics.md.
    type_arguments: ($) => seq('<', sepBy1(',', $._type), '>'),

    // A union runs as far as the `|`s go, so `fn() -> A|B` returns the union.
    // See `function_type`.
    union_type: ($) =>
      prec.right(1, seq($._atomic_type, repeat1(seq('|', $._atomic_type)))),

    // `[]` binds to the type on its left, so `fn(int) -> int[]` returns the
    // list. See `function_type`.
    array_type: ($) =>
      prec.right(1, seq(field('element', $._atomic_type), '[', ']')),

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
        field(
          'operator',
          choice('=', '+=', '-=', '*=', '/=', '%=', '^=', '&=', '|=', '^^=', '<<=', '>>='),
        ),
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
        // A name, plain or namespaced, is an expression at the same level as
        // everything that continues it: a call, a struct literal, a longer
        // path, a type argument list. Holding them level is what keeps each
        // reading a candidate until the token that settles it, so `a < b` and
        // `a<b>(c)` are both still in play at the `<`, and `Colour::Red` in an
        // `if` header is still a value and not the start of a literal.
        prec(PREC.postfix, $.identifier),
        prec(PREC.postfix, $.qualified_identifier),
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
        $.macro_invocation,
      ),

    parenthesized_expression: ($) => seq('(', $._expression, ')'),

    unary_expression: ($) =>
      prec.right(
        PREC.unary,
        seq(field('operator', choice('-', '!', '~')), field('operand', $._expression)),
      ),

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
        [PREC.bit_or, '|'],
        [PREC.bit_xor, '^^'],
        [PREC.bit_and, '&'],
        [PREC.shift, '<<'],
        [PREC.shift, '>>'],
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
    //
    // The form that names type arguments is a branch of its own so that it can
    // carry the dynamic precedence that settles `f(a < b, c > (d))`, which is
    // as good a comparison as it is a call.
    call_expression: ($) =>
      prec(
        PREC.postfix,
        choice(
          seq(field('function', $._callee), field('arguments', $.arguments)),
          prec.dynamic(
            PREC.type_arguments,
            seq(
              field('function', choice($.identifier, $.qualified_identifier)),
              field('type_arguments', $.type_arguments),
              field('arguments', $.arguments),
            ),
          ),
        ),
      ),

    // What a call applies to. A call attaches to the expression in front of it,
    // so what another call or an index handed back is called where it stands.
    // A type argument list only reads on a name, so that branch keeps its own
    // narrower callee.
    _callee: ($) =>
      choice(
        $.identifier,
        $.qualified_identifier,
        $.call_expression,
        $.index_expression,
        $.parenthesized_expression,
      ),

    method_call_expression: ($) =>
      prec(
        PREC.postfix,
        choice(
          seq(
            field('receiver', $._expression),
            '.',
            field('method', choice($.identifier, $.qualified_identifier)),
            field('arguments', $.arguments),
          ),
          prec.dynamic(
            PREC.type_arguments,
            seq(
              field('receiver', $._expression),
              '.',
              field('method', choice($.identifier, $.qualified_identifier)),
              field('type_arguments', $.type_arguments),
              field('arguments', $.arguments),
            ),
          ),
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
        prec(
          PREC.postfix,
          seq(
            field('name', choice($.identifier, $.qualified_identifier)),
            optional(field('type_arguments', $.type_arguments)),
            field('body', $.field_initializer_list),
          ),
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

    // `name!( ... )`, an expression the host that embeds candela expands. The
    // body is raw text in whatever language the host reads, so it is one
    // opaque token and none of candela's own rules apply inside it. `!(`
    // follows the name with nothing between, as it does in the lexer, so `!`
    // after a name with a space before the parenthesis stays the negation
    // operator.
    macro_invocation: ($) =>
      seq(
        field('name', $.identifier),
        token.immediate('!('),
        optional(field('body', $.macro_body)),
        ')',
      ),

    // A segment may name the instantiation it belongs to, which is how a
    // generic enum's variant is reached: `Slot<int>::Filled`, and
    // `g::Slot<int>::Filled` from a module bound with `as`.
    qualified_identifier: ($) =>
      prec(
        PREC.postfix,
        seq(
          field('module', choice($.identifier, $.qualified_identifier)),
          optional(field('type_arguments', $.type_arguments)),
          '::',
          field('name', $.identifier),
        ),
      ),

    // ------------------------------------------------------------------
    // Tokens
    // ------------------------------------------------------------------

    identifier: (_) => /[a-zA-Z_][a-zA-Z0-9_]*/,

    integer_literal: (_) => /[0-9]+/,

    // A leading digit is optional, so `.5` is a float; a trailing point is not.
    // An exponent makes a float with or without a point, as in `1e3`.
    float_literal: (_) =>
      /[0-9]*\.[0-9]+([eE][+-]?[0-9]+)?|[0-9]+[eE][+-]?[0-9]+/,

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
