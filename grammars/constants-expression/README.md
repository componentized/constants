# Expressions

An expression generates the items of a stream returned by an exported function, continuing from the items before them. Expressions are a subset of Rust's closures, e.g. `|a, b| a + b`.

This document specifies expressions fully, for tools such as language servers: where they appear, their lexical and syntactic grammar, names, types, evaluation, the rules for the functions declaring them, every diagnostic, and what can be completed at each position. A [tree-sitter grammar](#tree-sitter-grammar) implements the syntax.

For a shorter guide to writing expressions, see [docs/expressions.md](../../docs/expressions.md).

The reference implementation is [`crates/componentized-constants/src/expr.rs`](../../crates/componentized-constants/src/expr.rs). The grammar's test corpus is also run against it, so the two accept the same syntax.

## Contents <!-- omit in toc -->

- [Expressions](#expressions)
  - [Where expressions appear](#where-expressions-appear)
  - [Lexical grammar](#lexical-grammar)
  - [Syntax](#syntax)
  - [Names](#names)
  - [Types](#types)
  - [Evaluation](#evaluation)
  - [Declarations](#declarations)
  - [Diagnostics](#diagnostics)
    - [Lexical](#lexical)
    - [Syntax and names](#syntax-and-names)
    - [Types](#types-1)
    - [Declarations](#declarations-1)
  - [Completions](#completions)
  - [tree-sitter grammar](#tree-sitter-grammar)

## Where expressions appear

An expression follows an `@expression` tag in the doc comment of a function exported by the world, either directly or in an exported interface.

```wit
/// The Fibonacci sequence.
///
/// @value [1, 1]
/// @expression |a, b| a + b
fibonacci: async func() -> stream<u32>;
```

A doc comment's contents are its `///` lines with the `///` removed. Within the contents:

- A tag starts a line, after any whitespace, and is followed by whitespace or the end of the line. The tags are `@value` and `@expression`, so `@expressions` and `see @expression` aren't tags.
- A tag's text runs from the end of the tag to the start of the next line starting with a tag, or to the end of the contents. Text may span lines.
- The expression is the `@expression` tag's text with leading and trailing whitespace removed.
- A doc comment may have at most one of each tag, a second `@value` or `@expression` is an error.

The tags may appear in either order, after any other documentation.

Positions in diagnostics are `line:column` within the expression, both counted from 1 in characters, where line 1 starts at the expression's first character.

## Lexical grammar

The expression is a sequence of tokens separated by optional whitespace. There are no comments.

| Token       | Pattern                                                                                                                                                                 | Notes                                                                          |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| whitespace  | `[ \t\n\r\f]+`                                                                                                                                                          | ASCII only, ignored between tokens                                             |
| identifier  | `[A-Za-z][A-Za-z0-9_]*` or `_[A-Za-z0-9_]+`                                                                                                                             | ASCII only, except the keywords                                                |
| keyword | `if`, `else`, `true`, `false`, `let`, `mut`, `while`, `as` | reserved, never identifiers |
| variable | `$init`, `$call`, `$i` | `$` followed by an identifier, other names are errors |
| `_`         | `_`                                                                                                                                                                     | alone, it ignores a parameter or a `let` value                                 |
| integer     | `[0-9][0-9_]*`, `0x_*[0-9a-fA-F][0-9a-fA-F_]*`, `0o_*[0-7][0-7_]*`, `0b_*[01][01_]*`                                                                                    | decimal, hex, octal or binary                                                  |
| float       | `[0-9][0-9_]*\.([0-9][0-9_]*)?([eE][+-]?_*[0-9][0-9_]*)?` or `[0-9][0-9_]*[eE][+-]?_*[0-9][0-9_]*`                                                                      | decimal only, e.g. `1.5`, `1.`, `1e3`, `2.5E-3`                                |
| punctuation | `**%=` `<<%=` `>>%=` `**%` `**=` `<<%` `>>%` `<<=` `>>=` `+%=` `-%=` `*%=` `/%=` `%%=` `**` `\|\|` `&&` `<<` `>>` `<=` `>=` `==` `!=` `+%` `-%` `*%` `/%` `%%` `+=` `-=` `*=` `/=` `%=` `&=` `^=` `\|=` `+` `-` `*` `/` `%` `!` `&` `\|` `^` `<` `>` `=` `(` `)` `{` `}` `[` `]` `,` `;` `:` `.` | the longest match is taken, so `\|\|`, `+%` and `+=` are one token, and `+ %` is two |

Rules beyond the patterns:

- `_` separates digits and is otherwise ignored, so `1_000` is `1000`. A number can't start with `_`, which would be an identifier.
- A number can't be followed directly by an identifier character: literal suffixes such as `1u8` or `1.5f32` aren't supported. Use a cast, e.g. `1 as u8`, or a typed `let`.
- A `.` after decimal digits makes a float only when it isn't followed by another `.`, a `_` or a letter. So `1.` is a float, and in `$init.0.wrapping_add(1)` the `0` is an integer followed by `.`.
- Type names in casts and `let` annotations are identifiers: `bool`, `u8`, `u16`, `u32`, `u64`, `i8`, `i16`, `i32`, `i64`, `f32` and `f64` as in Rust, and `s8`, `s16`, `s32` and `s64` as in WIT.
- Any other character, including non-ASCII letters, is an error.

## Syntax

In EBNF, where `{ x }` repeats `x` zero or more times and `[ x ]` makes it optional:

```ebnf
closure        = ( "||" | "|" [ parameters ] "|" ) expression ;
parameters     = parameter { "," parameter } [ "," ] ;
parameter      = identifier | "_" ;

expression     = or ;
or             = and { "||" and } ;
and            = comparison { "&&" comparison } ;
comparison     = bit_or [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) bit_or ] ;
bit_or         = bit_xor { "|" bit_xor } ;
bit_xor        = bit_and { "^" bit_and } ;
bit_and        = shift { "&" shift } ;
shift          = additive { ( "<<" | ">>" | "<<%" | ">>%" ) additive } ;
additive       = multiplicative { ( "+" | "-" | "+%" | "-%" ) multiplicative } ;
multiplicative = cast { ( "*" | "/" | "%" | "*%" | "/%" | "%%" ) cast } ;
cast           = unary { "as" type } ;
unary          = ( "-" | "-%" | "!" ) unary | power ;
power          = postfix [ ( "**" | "**%" ) unary ] ;
postfix        = primary { "." method "(" [ expression ] ")" } ;
primary        = integer | float | "true" | "false" | identifier
               | ( "$init" | "$call" ) "." ( integer | identifier ) | "$i"
               | "(" expression ")" | if | block | array ;
array          = "[" expression { "," expression } [ "," ] "]" ;
if             = "if" expression block [ "else" ( block | if ) ] ;
block          = "{" { statement } [ expression ] "}" ;
statement      = let | assignment | while | if [ ";" ] ;
let            = "let" [ "mut" ] ( identifier | "_" ) [ ":" type ] "=" expression ";" ;
assignment     = identifier ( "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "<<=" | ">>=" | "&=" | "^=" | "|="
               | "+%=" | "-%=" | "*%=" | "/%=" | "%%=" | "<<%=" | ">>%=" | "**=" | "**%=" ) expression ";" ;
while          = "while" expression block [ ";" ] ;
method         = "wrapping_add" | "wrapping_sub" | "wrapping_mul"
               | "rotate_left" | "rotate_right" | "to_le_bytes" | "to_be_bytes" ;
type           = "bool" | "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64"
               | "s8" | "s16" | "s32" | "s64" | "f32" | "f64" ;
```

The whole expression must be one `closure`, with nothing after it. The field after `$init.` or `$call.` is a decimal integer without `_`, or a name. `to_le_bytes` and `to_be_bytes` take no argument, the other methods take one.

Operators by precedence, highest first, as in Rust:

| Precedence | Operators                   | Associativity                 |
| ---------- | --------------------------- | ----------------------------- |
| 1          | method calls `.method(…)`   | left                          |
| 2 | `**` `**%` | right, and tighter than a unary operator on its left, so `-x ** 2` is `-(x ** 2)` |
| 3          | unary `-` `-%` `!`          | prefix                        |
| 4          | `as`                        | left                          |
| 5          | `*` `/` `%` `*%` `/%` `%%`  | left                          |
| 6          | `+` `-` `+%` `-%`           | left                          |
| 7          | `<<` `>>` `<<%` `>>%`       | left                          |
| 8          | `&`                         | left                          |
| 9          | `^`                         | left                          |
| 10          | `\|`                        | left                          |
| 11         | `==` `!=` `<` `<=` `>` `>=` | none, `a < b < c` is an error |
| 12         | `&&`                        | left                          |
| 13         | `\|\|`                      | left                          |

So `-x.wrapping_add(1)` negates the call, `-x as u8` casts the negation, and `x as u64 * 2` multiplies the cast. `**`'s base is a method call or tighter, and its exponent a unary expression, so `2 ** 3 ** 2` is `2 ** 9`, `x ** -1` needs no parentheses, and a cast needs them, `(x as u64) ** 2`.

`**` raises to a power, as in Python, `**%` with wrapping. The wrapping operators, ending in `%` as in Zig, have the precedence of the operators they wrap, so `a +% b *% c` is `a +% (b *% c)`.

`if` and blocks are expressions, so they may be operands, e.g. `1 + if c { a } else { b }`. A block is statements followed by an optional value: `let` bindings, assignments, `while` loops and `if` statements. Where a block is a value, as an operand, a `let` value, an `if` expression's branch or the closure's body, it must end with a value, and an `if` must have an `else`, with a value for each branch. A `while` loop's body and an `if` statement's branches are statements only, without a value, and an `if` statement may leave out `else`. An `if` starting a statement is a statement, unless it's the last thing in its block and every branch has a value, when it's the block's value, as in Rust. `else if` chains conditions. The grammar accepts both shapes of blocks and `if`s everywhere, these rules are checked after parsing, see [Syntax and names](#syntax-and-names).

`|` both delimits the parameters and is the bitwise or operator. Only the first two `|` (or the first `||`) delimit the parameters, so `|a| a | 1` is valid. `| |` with whitespace is also a closure without parameters.

Not supported: `match`, `loop`, `for`, `break`, `continue`, `return`, statements other than `let`, assignment, `while` and `if`, assignment to anything but a `let mut` binding, other methods, method calls on literals, fields other than `$init`'s, type annotations on parameters such as `|a: u32|`, array repeats such as `[0; 4]`, indexing, ranges, tuples, strings, chars and comments.

## Names

An identifier in the body is a `let` binding, a closure parameter, or one of the function's arguments, the innermost first.

- **Parameters.** With `N` parameters, the next item is computed from the previous `N` items of the stream, oldest first: generating item `i`, the first parameter is item `i - N` and the last is item `i - 1`. A parameter is an identifier or `_`, which takes a position without a name and may appear more than once. Parameter names must be unique.
- **`let` bindings.** A binding is in scope from the end of its `let` to the end of its block. A later binding of the same name shadows an earlier binding or a parameter, as in Rust, so `let x = x ^ x << 13;` reads the previous `x`. `let _ = …;` evaluates its value without binding it. A `let mut` binding can be assigned, `x = value;`, or with a compound assignment, `x op= value;`, which is `x = x op value;`, for each binary arithmetic, bitwise, shift or wrapping operator. Other bindings and the parameters can't be assigned.
- **`$init`** holds the function's arguments, read by field: by index, `$init.0` is the first argument, `$init.1` the second, and so on, or by the parameter's name, as Rust names it, with `-` replaced by `_`, e.g. `$init.start_value` for a parameter `start-value`. It can only be used in a function with parameters, see [Declarations](#declarations).
- **`$call`** holds the closure's parameters, the arguments of the current evaluation, read by field: by index, `$call.0` is the first parameter, or by its name, `$call.a`. Unlike the parameter's name, `$call` can't be shadowed, so `$call.a` reads the parameter even after `let a = …;`, and `$call.0` reads a parameter ignored with `_`. It can be used in any expression, with as many fields as the closure has parameters.
- **Arguments.** The function's arguments are in scope by name, outside the closure's parameters, named as `$init`'s fields are, so `n` is `$init.n`. Reading one reads `$init`, so the arguments are inputs, not items, see [Declarations](#declarations). A closure parameter or `let` binding of the same name shadows an argument, so in `|a, b| a + b` for `func(a: u32, b: u32)`, `a` and `b` are the previous items. An argument is never assigned, even when its name is shadowed by `let mut`.
- **`$i`** is the position in the stream of the item being generated, counting from 0 and including the items the stream starts from. After `@value [1, 1]`, the first item generated has `$i` 2. When the closure returns an array, `$i` is the position of its first item, so it advances by the array's length: 0, 8, 16, … for `to_le_bytes` of a `u64`.

`$init`, `$call` and `$i` are built in: they can't be shadowed, by a parameter or `let`, nor assigned, and nothing else changes them. Operators and methods give new values, and a copy, `let mut x = $i;`, can change without changing `$i`. `$init` and the arguments are the same for every item, `$call` is the parameters of the current evaluation, and `$i` is the same throughout an evaluation, advancing only between items.

`if`, `else`, `true`, `false`, `let`, `mut`, `while` and `as` are keywords, so they can't be bound.

## Types

Every expression has a type: `bool`, `u8`, `s8`, `u16`, `s16`, `u32`, `s32`, `u64`, `s64`, `f32` or `f64`. Types are named in messages by their WIT names, so Rust's `i32` is `s32`.

The stream's item type `T` must be one of these, after following type aliases. The closure's body has type `T`, and so does each parameter. `$init.k` has the type of the function's `k`-th parameter, and `$i` is a `u64`.

Literals take the type their context needs. An integer literal can be any integer or float type, a float literal any float type. Where the context doesn't decide, e.g. both operands of a comparison are literals, or a literal is cast, an integer literal is an `s32` and a float literal an `f64`, as in Rust. So `0x8000_0001 as u32` is out of range for `s32`: write `let x: u32 = 0x8000_0001;` instead. A `let` bound to only literals must give its type.

| Expression                                               | Operands                                        | Type                     | Rules                                                              |
| -------------------------------------------------------- | ----------------------------------------------- | ------------------------ | ------------------------------------------------------------------ |
| integer literal                                          |                                                 | an integer or float type | in range for an integer type                                       |
| float literal                                            |                                                 | a float type             | finite as an `f32`                                                 |
| `true`, `false`                                          |                                                 | `bool`                   |                                                                    |
| parameter, `let` binding, `$init.k`, `$call.k`, `$i` | | its type | |
| `-e`                                                     | `e: X`                                          | `X`                      | `X` must be a signed integer or a float                            |
| `-%e`                                                    | `e: X`                                          | `X`                      | `X` must be an integer                                             |
| `!e`                                                     | `e: X`                                          | `X`                      | logical not for `bool`, bitwise not for an integer, not for floats |
| `a * b`, `a / b`, `a + b`, `a - b`                       | `X`, `X`                                        | `X`                      | `X` must be an integer or float                                    |
| `a % b`                                                  | `X`, `X`                                        | `X`                      | `X` must be an integer                                             |
| `a +% b`, `a -% b`, `a *% b`, `a /% b`, `a %% b`         | `X`, `X`                                        | `X`                      | `X` must be an integer                                             |
| `a ** b`, `a **% b` | `a: X`, `b`: any integer | `X` | `X` must be an integer or float, `**%` an integer, a literal exponent is a `u32`, or an `s32` for a float base |
| `a << b`, `a >> b`, `a <<% b`, `a >>% b`                 | `a: X`, `b`: any integer                        | `X`                      | `X` must be an integer, a literal amount takes `X`                 |
| `a & b`, `a ^ b`, `a \| b`                               | `X`, `X`                                        | `X`                      | `X` must be an integer or `bool`, `bool` without short-circuiting  |
| `a == b`, `a != b`, `a < b`, `a <= b`, `a > b`, `a >= b` | `X`, `X`                                        | `bool`                   | `false` is less than `true`                                        |
| `a && b`, `a \|\| b`                                     | `bool`, `bool`                                  | `bool`                   | short-circuiting                                                   |
| `e as Y`                                                 | `e`: a number, or `bool` when `Y` is an integer | `Y`                      | `Y` can't be `bool`, see [Evaluation](#evaluation)                 |
| `a.wrapping_add(b)`, `wrapping_sub`, `wrapping_mul`      | `X`, `X`                                        | `X`                      | `X` must be an integer                                             |
| `a.rotate_left(b)`, `rotate_right`                       | `a: X`, `b: u32`                                | `X`                      | `X` must be an integer                                             |
| `if c { a } else { b }`                                  | `c: bool`, `a` and `b` the same type            | the type of `a`          |                                                                    |
| `{ let x: Y = e; …; v }` | `e: Y`, or `e`'s type without `: Y` | the type of `v` | |
| `x = e;`, `x op= e;` | `e`, or `x op e`, the type of `x` | | a statement, `x` a `let mut` binding |
| `while c { … }` | `c: bool` | | a statement |
| `if c { … } else { … }` | `c: bool` | | a statement, when its branches have no values |
| `( e )`                                                  |                                                 | the type of `e`          |                                                                    |

The closure may instead return an array, whose elements are each an item, in order, so one evaluation generates several items. Arrays can only be the closure's result: the body, a block's value or an `if`'s branches in that position, where both branches must give the same number of items. Elsewhere, an array is an error.

| Result            | Items                                | Rules                                                               |
| ----------------- | ------------------------------------ | ------------------------------------------------------------------- |
| `[a, b, …]`       | each element, at least one           | each element must be the item type `T`                              |
| `x.to_le_bytes()` | `x`'s bytes, least significant first | `T` must be `u8`, `x` any integer or float, as Rust's `to_le_bytes` |
| `x.to_be_bytes()` | `x`'s bytes, most significant first  | as `to_le_bytes`                                                    |

Operands of the same type must have exactly the same type, there are no implicit conversions: `$init.0 * $i` with a `u32` seed is an error, `$init.0 as u64 * $i` isn't.

A `-` applied directly to a literal, even through parentheses, makes a negative literal, so `-128` is in range for `s8`. For an unsigned type, a negative literal is out of range.

## Evaluation

A stream with an expression writes the items it starts from, see [Declarations](#declarations), then items generated from them, one at a time, until an item can't be generated. When the reader closes the stream, nothing more is generated. A stream whose expression never fails is unbounded. Each call returns a new stream, from the same items and arguments, so the same call always generates the same items.

Operands, `let` values, statements and array elements are evaluated in order, left to right. An array's items are generated together: if any element fails, the stream ends before the array's first item, so the items written are always whole arrays. `&&`, `||` and `if` evaluate only what they need, so a failure in an operand or branch that isn't evaluated doesn't end the stream. A `let` value that fails ends the stream, even when the binding isn't used.

A `while` loop evaluates its condition, then its body, until the condition is `false`. A failure inside a loop ends the stream like any other, which ends most runaway loops: a counter that only grows overflows. A loop whose condition never becomes `false` without failing, e.g. `while true { }`, never ends, and nor does the stream's task, as in Rust, so the reader waits forever for the next item.

An item can't be generated, and the stream ends without it, when evaluating the expression would fail Rust's checked arithmetic, or a float result isn't finite:

| Operation                                                                 | Integers                                                | Floats                                                    |
| ------------------------------------------------------------------------- | ------------------------------------------------------- | --------------------------------------------------------- |
| `a + b`, `a - b`, `a * b`                                                 | the result is out of the type's range                   | the result is infinite or NaN                             |
| `a ** b` | the result is out of the type's range, or the exponent is negative | the result is infinite or NaN, a negative exponent gives the reciprocal |
| `a / b`                                                                   | `b` is zero, or `a` is the signed minimum and `b` is -1 | the result is infinite or NaN, including when `b` is zero |
| `a % b`                                                                   | `b` is zero, or `a` is the signed minimum and `b` is -1 | not supported                                             |
| `-a`                                                                      | `a` is the signed minimum                               | never fails                                               |
| `a << b`, `a >> b`                                                        | `b`, read as unsigned, is at least the type's bits      | not supported                                             |
| `a /% b`, `a %% b`                                                        | `b` is zero                                             | not supported                                             |
| `!`, `&`, `^`, `\|`, comparisons, `as`, methods, other wrapping operators | never fail                                              | never fail, comparisons follow IEEE 754                   |

The stream also ends after the item at position `u64::MAX`, as `$i` can't go further, which never happens in practice.

Shifts only check the amount: bits shifted past the type's width are dropped, as in Rust, so `1u32 << 31 << 1` is `0`. `>>` is arithmetic for signed types and logical for unsigned types.

The wrapping operators wrap around the type's range instead of failing, as Rust's `wrapping_*` methods:

| Operator                     | As Rust's                                      | Result                                                                               |
| ---------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------------------ |
| `a +% b`, `a -% b`, `a *% b` | `wrapping_add`, `wrapping_sub`, `wrapping_mul` | the result modulo 2 to the type's bits                                               |
| `a /% b`, `a %% b`           | `wrapping_div`, `wrapping_rem`                 | as `/` and `%`, except the signed minimum and -1 give the minimum and 0              |
| `a <<% b`, `a >>% b`         | `wrapping_shl`, `wrapping_shr`                 | a shift by `b` modulo the type's bits, e.g. `x <<% 33` shifts a `u32` by 1           |
| `a **% b` | `wrapping_pow` | the result modulo 2 to the type's bits |
| `-%a`                        | `wrapping_neg`                                 | 0 minus `a`, wrapping, so it's allowed for unsigned types, e.g. `-%(1 as u8)` is 255 |

The methods `wrapping_add`, `wrapping_sub` and `wrapping_mul` are the same as `+%`, `-%` and `*%`. `rotate_left` and `rotate_right` rotate the type's bits by the amount modulo the type's bits.

Casts with `as` never fail, following Rust:

| From    | To      | Result                                                                                                  |
| ------- | ------- | ------------------------------------------------------------------------------------------------------- |
| integer | integer | truncated to the target's bits, or extended with the source's sign or zeros, e.g. `-3 as u8` is 253     |
| `bool`  | integer | 0 or 1                                                                                                  |
| integer | float   | the nearest float                                                                                       |
| float   | float   | rounded to `f32`, or exact to `f64`                                                                     |
| float   | integer | rounded toward zero, saturating at the target's range, with NaN becoming 0, e.g. `-150.0 as i8` is -128 |

## Declarations

A function with an `@expression` must return a `stream<T>`, possibly through type aliases, where `T` is one of the types above. The function must be `async`, since its stream is written after the call returns, and may be unbounded. It takes one of three forms.

**Listed items.** The function takes no parameters. Its `@value` tag lists the items the stream starts from, which must number at least as many as the closure's parameters. The closure starts from the last of them. The `@value` is required, though it may be `[]` when the closure takes no parameters. An override replaces the listed items, but not the expression. `$init` can't be used.

```wit
/// @value [1, 1]
/// @expression |a, b| a + b
fibonacci: async func() -> stream<u32>;
```

**Arguments as items.** The expression doesn't read `$init`, or an argument by name. The function takes the items the stream starts from as parameters, one for each of the closure's, by position, each of type `T`. Names may differ from the closure's. The arguments are the stream's first items. The function has no `@value`.

```wit
/// @expression |a, b| a + b
fibonacci: async func(a: u32, b: u32) -> stream<u32>;
```

**Arguments as `$init`.** The expression reads `$init`, or an argument by name. The function's parameters may have any of the types above, and are only read through `$init`, they aren't items. The stream starts from the items listed by an optional `@value`, which must number at least as many as the closure's parameters, and are none without a `@value`.

```wit
/// @expression || {
///     let z = $init.0.wrapping_add($i.wrapping_add(1).wrapping_mul(0x9E3779B97F4A7C15));
///     let z = (z ^ z >> 30).wrapping_mul(0xBF58476D1CE4E5B9);
///     let z = (z ^ z >> 27).wrapping_mul(0x94D049BB133111EB);
///     z ^ z >> 31
/// }
pseudorandom: async func(seed: u64) -> stream<u64>;
```

A function with parameters can't be overridden, and takes at most 16 parameters. Without an `@expression`, an exported function can't take parameters.

## Diagnostics

Diagnostics found while creating a component are reported for the function, e.g. ``invalid `@expression` for `fibonacci` at world.wit:4:5: unknown name `c` at 1:12``. Diagnostics with a position report it as `at line:column` within the expression, see [Where expressions appear](#where-expressions-appear), or `at the end` when the expression ends early. Type diagnostics have no position, so a language server should place them on the expression they describe.

In the messages below, `T`, `X` and `Y` are types by their WIT names, e.g. `u32` or `s8`, `op` is an operator, `n` and `k` are numbers, and `at` is a position, followed by the token found there.

### Lexical

| Message                                | Cause                                                                    |
| -------------------------------------- | ------------------------------------------------------------------------ |
| ``unexpected `c` at``                  | a character that starts no token, including a `$` not followed by a name |
| `invalid integer at`                   | a prefix without digits, e.g. `0x`, or an integer too large for 128 bits |
| `invalid float at`                     | an exponent without digits, e.g. `1e`                                    |
| `literal suffixes aren't supported at` | a number followed by an identifier character, e.g. `1u8`                 |

### Syntax and names

| Message                                                                                          | Cause                                                                            |
| ------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| ``expected `\|` at``                                                                             | the expression doesn't start with a closure's parameters                         |
| `expected a parameter name at`                                                                   | a parameter that isn't an identifier or `_`, including a keyword                 |
| ``expected `,` or `\|` at``                                                                      | parameters not separated by `,` or ended by `\|`                                 |
| ``duplicate parameter `name` at``                                                                | a parameter name used twice                                                      |
| `expected an expression at`                                                                      | a token that can't start an expression, or the end, where an operand is expected |
| ``expected `)` at``, ``expected `}` at``, ``expected `(` at``                                    | an unclosed parenthesis or block, or a method without its argument               |
| ``expected `{` at``, ``expected `{` or `if` at``                                                 | an `if` condition or `else` without a block                                      |
| ``expected `else`, an `if` needs a value for both branches at`` | an `if` as a value without `else` |
| `expected a value at the end of the block at` | a block as a value ending with a statement, e.g. a `let` |
| ``a `while` loop's or an `if` statement's block can't end with a value at`` | a value at the end of a loop's body or an `if` statement's branch |
| ``expected `}`, an `if` with values must be the block's value at`` | an `if` with values starting a statement that continues, e.g. `if c { 1 } else { 2 } + 1` |
| ``can't assign to `x` at …, declare it with `let mut x` `` | an assignment to a `let` binding without `mut` |
| ``can't assign to the parameter `x` at …, bind a copy with `let mut x = x;` `` | an assignment to a parameter |
| ``can't assign to the argument `x` at …, arguments never change, bind a copy with `let mut x = x;` `` | an assignment to one of the function's arguments |
| ``` `$i` can't be bound at …, `$init`, `$call` and `$i` are built in, and can't be shadowed ``` | `$init`, `$call` or `$i` as a parameter or `let` name |
| ``` `$i` can't be assigned at …, `$init`, `$call` and `$i` are built in, and never change, bind a copy with `let mut` ``` | an assignment to `$i`, or a field of `$init` or `$call` |
| `comparison operators can't be chained at`                                                       | a comparison as the operand of another, e.g. `a < b < c`                         |
| `expected a name to bind at`                                                                     | a `let` without an identifier or `_`, including a keyword                        |
| ``expected `=` at``, ``expected `;` at`` | a `let` without a value, or a `let` or assignment not ended by `;` |
| ``expected a type, e.g. `u8`, `i32` or `f64`, at``                                               | an `as` or `let` annotation without a type name                                  |
| `expected a method name at`                                                                      | a `.` not followed by a name, e.g. `n.0`                                         |
| ``unknown method `name` at …, the methods are …``                                                | a method other than the seven supported                                          |
| ``` `name` takes an argument at ```, ``` `name` takes no arguments at ```                        | a method called with the wrong number of arguments                               |
| `an array must have at least one item at`                                                        | `[]`                                                                             |
| ``expected `,` or `]` at``                                                                       | array elements not separated by `,` or ended by `]`                              |
| ``can't call `name` on a literal at …, its type is ambiguous``                                   | a method call on a literal, e.g. `1.wrapping_add(2)`                             |
| ``expected `.` and a field, `$init` is a tuple of the function's arguments, e.g. `$init.0`, at`` | `$init` without a field                                                          |
| ``expected a field of `$init`, an index, e.g. `0`, or a parameter's name, at`` | a `$init.` not followed by a decimal integer or a name |
| ``expected `.` and a field, `$call` is the closure's parameters, e.g. `$call.0`, at`` | `$call` without a field |
| ``expected a field of `$call`, an index, e.g. `0`, or a parameter's name, at`` | a `$call.` not followed by a decimal integer or a name |
| ``` `$call.k` doesn't exist at …, `$call` has n fields, one for each of the closure's parameters ``` | a field past the closure's parameters |
| ``` `$call` has no field `name` at …, the closure's parameters are … ``` | a field by a name that isn't one of the closure's parameters |
| ``unknown variable `$name` at …, the variables are `$init`, `$call` and `$i` `` | any other `$` name |
| ``unknown name `name` at`` | an identifier that isn't a parameter, a `let` binding in scope, or an argument |
| `literal n is too large at`                                                                      | an integer literal too large for any supported type                              |
| ``unexpected `token` at``                                                                        | tokens after the closure's body                                                  |

### Types

| Message                                                                                         | Cause                                                                                         |
| ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `expected the closure to return T, found X`                                                     | the body's type isn't the item type                                                           |
| `expected X, found Y`                                                                           | a parameter, binding, `$init.k`, `$i` or cast of type `Y` where `X` is needed                 |
| `expected bool, found integer n`                                                                | an integer literal where a `bool` is needed                                                   |
| `literal n is out of range for X`                                                               | an integer literal outside `X`'s range, or a float literal infinite as `f32`                  |
| `expected X, found float n`                                                                     | a float literal where an integer or `bool` is needed                                          |
| ``mismatched types for `op`, X and Y``                                                          | operands of different types, e.g. `n + true`, also for methods, `if` branches and comparisons |
| ``expected X for the `else` branch, found Y``                                                   | `if` branches of different types                                                              |
| ``expected bool for an `if` condition, found X``, ``expected bool for a `while` condition, found X`` | a condition that isn't a `bool` |
| `expected X for an assignment, found Y` | an assigned value of another type than the binding |
| ``` `-` can't negate X ```                                                                      | `-` on an unsigned type                                                                       |
| ``` `!` isn't supported for X ```                                                               | `!` on a float                                                                                |
| ``` `-%` isn't supported for X ```                                                              | `-%` on a float or `bool`                                                                     |
| ``` `op` expects bool operands ```                                                              | `&&` or `\|\|` on operands that aren't `bool`                                                 |
| ``` `op` expects numeric operands ```                                                           | arithmetic or shifts on `bool` operands                                                       |
| ``` `op` isn't supported for X ```                                                              | `%`, `<<`, `>>`, `&`, `^` or `\|` on floats, or a wrapping operator on a float or `bool`      |
| ``the exponent of `**` must be an integer, found X`` | a float or `bool` exponent |
| `a shift amount must be an integer, found X`                                                    | a shift by a float or `bool`                                                                  |
| ``` `method` isn't supported for X ```                                                          | a method on a float or `bool`                                                                 |
| ``can't cast to bool, compare instead, e.g. `x != 0` ``                                         | `as bool`                                                                                     |
| `can't cast bool to X`                                                                          | `as` from `bool` to a float                                                                   |
| ``the type of a `let` bound to a literal must be given, e.g. `let x: u32 = 1;` ``               | a `let` without a type, whose value is only literals                                          |
| ``an array can only be the closure's result, e.g. `\|\| [a, b]` or `\|\| x.to_le_bytes()` ``    | an array, or bytes, as an operand or `let` value                                              |
| ``the `if` branches give n and m items, they must give the same number``                        | `if` branches returning arrays of different lengths, or an array and a single item            |
| ``` `to_le_bytes` gives bytes, the stream's items must be u8, not T ```                         | bytes in a stream of another type                                                             |
| ``` `to_le_bytes` isn't supported for bool ```                                                  | bytes of a `bool`                                                                             |
| ``` `$init.k` doesn't exist, `$init` has n fields, one for each of the function's arguments ``` | a field past the function's parameters                                                        |
| ``` `$init` has no field `name` at …, the function's parameters are … ``` | a field by a name that isn't one of the function's parameters |

### Declarations

| Message                                                                                                                      | Cause                                                                           |
| ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| ``function `f`: duplicate `@expression` tag, a doc comment may have one of each tag``                                        | a second `@expression`, or `@value`, tag in the doc comment                     |
| `only functions returning a stream can generate items`                                                                       | an `@expression` on a function not returning a stream                           |
| `only streams of integers, floats or bool can generate items`                                                                | a stream of another item type                                                   |
| ``a stream with items must be returned by an async function, e.g. `async func() -> stream<u8>` ``                            | a listed-items function that isn't `async`                                      |
| ``a stream generated from arguments must be returned by an async function, e.g. `async func(n: u8) -> stream<u8>` ``         | a function with parameters that isn't `async`                                   |
| `the expression reads the previous n items, list at least n`                                                                 | fewer listed items than the closure's parameters                                |
| ``the expression reads the previous n items, list at least n with `@value` ``                                                | a `$init` function without a `@value`, whose closure has parameters             |
| ``missing value for `f` at …: add a `@value` tag to its doc comment, or an override``                                        | a listed-items function without a `@value`                                      |
| `the expression takes n parameters, the function takes m parameters`                                                         | an arguments-as-items function whose parameter count differs from the closure's |
| ``parameter `p` must have the stream's item type, `T` ``                                                                     | an argument as an item that isn't a `T`                                         |
| ``` `$init` holds the function's arguments, but the function takes none ```                                                  | `$init` in a function without parameters                                        |
| ``parameter `p` can't be read with `$init`, its type must be an integer, float or bool``                                     | a `$init` function with a parameter of another type                             |
| `a stream can be generated from at most 16 arguments`                                                                        | more than 16 parameters                                                         |
| ``invalid `@value` for `f` at …: its stream starts from its arguments, remove the tag, or read the arguments with `$init` `` | an arguments-as-items function with a `@value`                                  |
| ``invalid overrides: `f` can't be overridden, its stream is generated from its arguments``                                   | an override for a function with parameters                                      |
| ``function `f` must not accept parameters, unless it generates a stream from them with an `@expression` ``                   | parameters without an `@expression`                                             |

## Completions

What can be completed depends on the position and, for operands, on the type the context expects:

| Position                                                                                         | Expected type                                  |
| ------------------------------------------------------------------------------------------------ | ---------------------------------------------- |
| the closure's body                                                                               | `T`                                            |
| an `if` condition                                                                                | `bool`                                         |
| an `if` branch, a block's value                                                                  | the type the `if` or block is expected to have |
| a `let` value                                                                                    | its annotated type, otherwise any              |
| an operand of `&&`, `\|\|`                                                                       | `bool`                                         |
| an operand of arithmetic, wrapping operators, `&`, `^`, `\|`, `-`, `-%`, `!`, or a shifted value | the expected type, matching the other operand  |
| a shift amount                                                                                   | any integer                                    |
| an operand of a comparison, or the value of a cast                                               | any, matching the other operand                |
| a `wrapping_*` method's receiver and argument                                                    | the expected type                              |
| a rotation's receiver, its amount                                                                | the expected type, `u32`                       |
| inside `( )`                                                                                     | the type the parentheses are expected to have  |

Completions by position:

- **A doc comment line**: the tags `@value` and `@expression` that the comment doesn't have yet.
- **The start of an expression**: a closure with the parameters it needs. For an arguments-as-items function, its parameter names, e.g. `|a, b| `. Otherwise any names, with at most as many parameters as listed items, or `|| ` for a `$init` function without a `@value`.
- **A parameter**: for an arguments-as-items function, the function's parameter name at the same position, or `_`.
- **An operand**: by expected type, the parameters, `let` bindings and arguments in scope of that type, `$init.k` for each argument of that type, `$call.k` for each parameter when that's the item type, `$i` for `u64`, a literal as the type allows, `(`, `{`, `if`, `-` for a signed integer or float, `-%` for an integer, and `!` for an integer or `bool`. Any value followed by `as` reaches a numeric type, so values of other numeric types are completions too, with `as` and the type. An expected `bool` can also start a comparison, so operands of any type are completions too, as can `true` and `false`.
- **After `$init`**: `.` and the fields, by name and by index, `0` up to the number of arguments, with their types.
- **After `$call`**: `.` and the fields, by name and by index, `0` up to the number of the closure's parameters, of the item type.
- **After a complete operand of type `X`**: `.` and the methods for an integer, `as` and the types it can be cast to, and the binary operators valid for `X` and the expected type, e.g. after a `u32`, `+ - * / % << >> & ^ |`, the wrapping operators `+% -% *% /% %% <<% >>%`, `**` and `**%`, and the comparisons, plus `&&` and `||` after a `bool`. Then `)` inside parentheses, or `}` inside a block. After an `if` condition, `{`. After an `if`'s first block, `else`. After `else`, `{` or `if`.
- **The start of a block, or after a statement**: `let`, `while`, `if`, the `let mut` bindings in scope followed by the assignment operators valid for their type, and the operands for the block's value, where it has one.
- **The closure's result**: also `[`, and after a value, `.to_le_bytes()` and `.to_be_bytes()` when the items are `u8`.
- **After `let`**: `mut`, or a name. After a name, `:` and the types, or `=`. After an assignment's or `let`'s value, `;`. After a `while` condition, `{`.
- Don't offer a comparison operator after a comparison, since they don't chain.

Literal hints can show the expected type's range, e.g. `0..=255` for `u8`, as literals outside it are errors.

## tree-sitter grammar

[`grammars/constants-expression`](../../grammars/constants-expression) is a [tree-sitter](https://tree-sitter.github.io) grammar for the [syntax](#syntax) of a single expression, the text of an `@expression` tag. It parses a whole closure, recovering from errors with `ERROR` and `MISSING` nodes, so editors can report syntax errors and find the node at the cursor. Names, types and declarations are checked separately, as described above.

| Node                                                                         | Fields                                                                             | Children                               |
| ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | -------------------------------------- |
| `closure`                                                                    | `parameters: parameters` (absent for `\|\|`), `body`                               |                                        |
| `parameters`                                                                 |                                                                                    | `parameter`s                           |
| `parameter`                                                                  |                                                                                    | an `identifier`, or the `_` token      |
| `binary_expression`                                                          | `left`, `operator` (the operator token), `right`                                   |                                        |
| `unary_expression`                                                           | `operator` (`-` or `!`), `operand`                                                 |                                        |
| `cast_expression`                                                            | `value`, `type: primitive_type`                                                    |                                        |
| `method_call`                                                                | `receiver`, `method: identifier`, `argument` (absent without one)                  |                                        |
| `array_expression`                                                           |                                                                                    | the elements                           |
| `builtin_field` | `variable` (the `$init` or `$call` token), `field: field_index`, or `field: identifier` for a name | |
| `index`                                                                      |                                                                                    |                                        |
| `if_expression` | `condition`, `consequence: block`, `alternative: block` or `if_expression` (absent without `else`) | |
| `block` | | statements, `let_declaration`, `assignment_statement`, `while_statement` or `if_expression`, then an optional expression |
| `let_declaration` | `pattern: identifier` (absent for `_`), `type: primitive_type` (optional), `value` | `mutable` for `let mut` |
| `assignment_statement` | `left: identifier`, `operator`, `right` | |
| `while_statement` | `condition`, `body: block` | |
| `parenthesized_expression`                                                   |                                                                                    | an expression                          |
| `identifier`, `integer`, `float`, `boolean`, `primitive_type`, `field_index` |                                                                                    |                                        |

A negative literal parses as a `unary_expression` of `-` and the literal. A block's final `if` parses the same whether it's a statement or the block's value.

[`queries/highlights.scm`](./queries/highlights.scm) highlights expressions. To highlight them in WIT, a WIT grammar can inject the `constants_expression` language into the text following an `@expression` tag in a doc comment, extracted as described in [Where expressions appear](#where-expressions-appear).

`make test-grammar` generates the parser with the tree-sitter CLI pinned in [`tools/Cargo.toml`](../../tools/Cargo.toml), using its built-in JavaScript runtime, then tests it against [`test/corpus`](./test/corpus). The corpus is also parsed by the reference implementation, which must accept every test and reject every `:error` test. Add cases to the corpus when changing the syntax, so the grammar and the implementation stay in step.
