/**
 * @file Expressions generating the items of a stream, a subset of Rust's closures
 * @license Apache-2.0
 *
 * Mirrors the parser in crates/componentized-constants/src/expr.rs, see
 * README.md. Each precedence level is its own rule, lowest first,
 * which makes comparisons non-associative as in Rust: `a < b < c` is an error.
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

/**
 * Binary operators by precedence, lowest first, each level's operands are the
 * next level's. A level is a hidden rule choosing between the next level and a
 * binary expression, a hidden rule of its own, aliased to `binary_expression`.
 */
const LEVELS = [
  ['or', ['||']],
  ['and', ['&&']],
  // comparisons don't chain, both operands are the next level
  ['comparison', ['==', '!=', '<', '<=', '>', '>='], {nonAssociative: true}],
  ['bit_or', ['|']],
  ['bit_xor', ['^']],
  ['bit_and', ['&']],
  // `%` suffixed operators wrap around the type's range, as in Zig
  ['shift', ['<<', '>>', '<<%', '>>%']],
  ['additive', ['+', '-', '+%', '-%']],
  ['multiplicative', ['*', '/', '%', '*%', '/%', '%%']],
];

/** @param {string[]} operators */
const operator = operators => operators.length === 1 ? operators[0] : choice(...operators);

/** The rules for each level, `_<level>` and `_<level>_expression`. */
const levelRules = Object.fromEntries(LEVELS.flatMap(([name, operators, options], index) => {
  const level = `_${name}`;
  const expression = `_${name}_expression`;
  const next = index + 1 < LEVELS.length ? `_${LEVELS[index + 1][0]}` : '_cast';
  return [
    [level, $ => choice($[next], alias($[expression], $.binary_expression))],
    [expression, $ => seq(
      field('left', options?.nonAssociative ? $[next] : $[level]),
      field('operator', operator(operators)),
      field('right', $[next]),
    )],
  ];
}));

module.exports = grammar({
  name: 'constants_expression',

  // ASCII whitespace
  extras: _ => [/[ \t\n\r\f]/],

  word: $ => $.identifier,

  reserved: {
    global: _ => ['if', 'else', 'true', 'false', 'let', 'as', 'while', 'mut'],
  },

  rules: {
    // `|a, b| a + b`, or `|| 7` without parameters
    closure: $ => seq(
      choice(
        '||',
        seq('|', optional(field('parameters', $.parameters)), '|'),
      ),
      field('body', $._expression),
    ),

    parameters: $ => seq(
      $.parameter,
      repeat(seq(',', $.parameter)),
      optional(','),
    ),

    // `_` ignores a parameter
    parameter: $ => choice($.identifier, '_'),

    _expression: $ => $._or,

    ...levelRules,

    // `x as u8`, binding tighter than binary operators and looser than unary
    _cast: $ => choice($._unary, $.cast_expression),

    cast_expression: $ => seq(
      field('value', $._cast),
      'as',
      field('type', $.primitive_type),
    ),

    _unary: $ => choice($.unary_expression, $._power),

    // `a ** b`, binding tighter than a unary operator on its left, and right
    // associative, as the exponent is a unary expression
    _power: $ => choice($._postfix, alias($._power_expression, $.binary_expression)),

    _power_expression: $ => seq(
      field('left', $._postfix),
      field('operator', choice('**', '**%')),
      field('right', $._unary),
    ),

    unary_expression: $ => seq(
      field('operator', choice('-', '-%', '!')),
      field('operand', $._unary),
    ),

    _postfix: $ => choice($._primary, $.method_call),

    // `x.wrapping_mul(y)` or `x.to_le_bytes()`, the methods and their
    // arguments are checked by name
    method_call: $ => seq(
      field('receiver', $._postfix),
      '.',
      field('method', $.identifier),
      '(',
      optional(field('argument', $._expression)),
      ')',
    ),

    _primary: $ => choice(
      $.integer,
      $.float,
      $.boolean,
      $.identifier,
      $.builtin_field,
      $.index,
      $.parenthesized_expression,
      $.if_expression,
      $.block,
      $.array_expression,
    ),

    // `[a, b, c]`, only valid as the closure's result
    array_expression: $ => seq(
      '[',
      $._expression,
      repeat(seq(',', $._expression)),
      optional(','),
      ']',
    ),

    // `$init.0` or `$init.name`, a field of the function's arguments, or `$call.0`
    // or `$call.name`, of the closure's parameters, by index or by name
    builtin_field: $ => seq(
      field('variable', choice('$init', '$call')),
      '.',
      field('field', choice($.field_index, $.identifier)),
    ),

    field_index: _ => /[0-9]+/,

    // `$i`, the position of the item being generated
    index: _ => '$i',

    primitive_type: _ => choice(
      'bool', 'u8', 'u16', 'u32', 'u64', 'i8', 'i16', 'i32', 'i64',
      's8', 's16', 's32', 's64', 'f32', 'f64',
    ),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    // `if c { a } else { b }`, both branches are required
    if_expression: $ => seq(
      'if',
      field('condition', $._expression),
      field('consequence', $.block),
      optional(seq('else', field('alternative', choice($.block, $.if_expression)))),
    ),

    mutable: _ => 'mut',

    // `{ let a = x; let b: u32 = y; a + b }`, statements then an optional value
    block: $ => seq('{', repeat($._statement), optional($._expression), '}'),

    _statement: $ => choice(
      $.let_declaration,
      $.assignment_statement,
      $.while_statement,
      // an `if` starting a statement is the whole statement, as in Rust, a final `if`
      // with values is the block's value, which the tree doesn't distinguish
      prec(1, seq($.if_expression, optional(';'))),
    ),

    // `x = y;` or a compound assignment, e.g. `x += 1;`
    assignment_statement: $ => seq(
      field('left', $.identifier),
      field('operator', choice(
        '=', '+=', '-=', '*=', '/=', '%=', '<<=', '>>=', '&=', '^=', '|=',
        '+%=', '-%=', '*%=', '/%=', '%%=', '<<%=', '>>%=', '**=', '**%=',
      )),
      field('right', $._expression),
      ';',
    ),

    while_statement: $ => seq(
      'while',
      field('condition', $._expression),
      field('body', $.block),
      optional(';'),
    ),

    let_declaration: $ => seq(
      'let',
      optional($.mutable),
      field('pattern', choice($.identifier, '_')),
      optional(seq(':', field('type', $.primitive_type))),
      '=',
      field('value', $._expression),
      ';',
    ),

    boolean: _ => choice('true', 'false'),

    // decimal, hex, octal or binary, digits may be separated by `_`, without a suffix
    integer: _ => token(choice(
      /[0-9][0-9_]*/,
      /0x_*[0-9a-fA-F][0-9a-fA-F_]*/,
      /0o_*[0-7][0-7_]*/,
      /0b_*[01][01_]*/,
    )),

    // `1.5`, `1.`, `1e3` or `1.5e-3`, without a suffix
    float: _ => token(choice(
      /[0-9][0-9_]*\.([0-9][0-9_]*)?([eE][+-]?_*[0-9][0-9_]*)?/,
      /[0-9][0-9_]*[eE][+-]?_*[0-9][0-9_]*/,
    )),

    identifier: _ => /[A-Za-z][A-Za-z0-9_]*|_[A-Za-z0-9_]+/,
  },
});
