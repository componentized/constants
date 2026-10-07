# Expressions

An `@expression` generates the items of a stream returned by an exported function. It's a closure written in a subset of Rust, with a few additions, returning the stream's next item each time it's evaluated.

This guide summarizes how to declare generated streams and what expressions can do. [grammars/constants-expression](../grammars/constants-expression/README.md) specifies expressions in full: their grammar, typing, evaluation and every diagnostic, with a tree-sitter grammar for editors and language servers.

- [Generating a stream](#generating-a-stream)
- [Built-in variables](#built-in-variables)
- [Syntax](#syntax)
- [Types](#types)
- [When a stream ends](#when-a-stream-ends)
- [Several items at once](#several-items-at-once)

## Generating a stream

A function with an `@expression` must be `async` and return a `stream<T>` of integers, floats or `bool`. Its items come from one of three places.

**Listed items.** The closure's parameters are the stream's previous items, oldest first. The stream starts with the items `@value` lists, at least one for each parameter, and the closure continues from the last of them. Overrides replace the listed items, but not the expression.

```wit
/// @value [1, 1]
/// @expression |a, b| a + b
fibonacci: async func() -> stream<u64>;
```

**Arguments as items.** The function takes the items the stream starts from as arguments, one for each of the closure's parameters, of the stream's item type. `fibonacci(1, 1)` streams `1, 1, 2, 3, 5, …`.

```wit
/// @expression |a, b| a + b
fibonacci: async func(a: u64, b: u64) -> stream<u64>;
```

**Arguments as inputs.** The expression reads the function's arguments by name, or through `$init`. They're inputs, not items, so they can have any integer, float or `bool` type. The stream starts from an optional `@value`, or from nothing.

```wit
/// @expression || n * r ** $i
geometric: async func(n: u64, r: u64) -> stream<u64>;
```

A function with arguments can't be overridden.

## Built-in variables

| Variable | Holds | Example |
| --- | --- | --- |
| `$init` | the arguments of the call that started the stream, the same for every item, by index or by name | `$init.0`, `$init.n` |
| `$call` | the closure's parameters, the arguments of the current evaluation, by index or by name | `$call.0`, `$call.a` |
| `$i` | the position in the stream of the item being generated, a `u64` | `$i` |

Arguments can also be read directly by name, e.g. `n`, and closure parameters by their names. Those names can be shadowed by a `let`, or an argument by a closure parameter, but `$init`, `$call` and `$i` can't be. None of them ever change. Names with `-` in WIT use `_`: an argument `start-value` is `start_value`.

## Syntax

Expressions follow Rust's syntax and precedence, unless noted:

- **Literals**: integers, in decimal, hex, octal or binary, floats, and `true` and `false`. Literals have no suffixes, they take their type from their context.
- **Arithmetic**: `+`, `-`, `*`, `/` and `%` (not for floats), and `**` for powers, as in Python, e.g. `r ** $i`, binding tighter than `*`, and associating right.
- **Wrapping arithmetic**, as in Zig: `+%`, `-%`, `*%`, `/%`, `%%`, `**%`, `<<%`, `>>%` and unary `-%` wrap around the type's range instead of ending the stream.
- **Bitwise and logical**: `&`, `|`, `^`, `<<`, `>>`, `!`, `&&` and `||`.
- **Comparisons**: `==`, `!=`, `<`, `<=`, `>` and `>=`, which don't chain.
- **Casts**: `as`, e.g. `$i as u8`, which truncate and saturate as in Rust.
- **Methods**: `wrapping_add`, `wrapping_sub`, `wrapping_mul`, `rotate_left` and `rotate_right`, and `to_le_bytes` and `to_be_bytes`.
- **Blocks**: `{ let a = …; a + 1 }`. `let mut` bindings can be assigned, `x = …;`, or with a compound assignment, `x += …;`, for any binary operator.
- **Control flow**: `if c { a } else { b }`, as a value or a statement, and `while` loops:

  ```wit
  /// @value [2]
  /// @expression |v| {
  ///     let mut n = v + 1;
  ///     let mut d: u64 = 2;
  ///     while d * d <= n {
  ///         if n % d == 0 {
  ///             n += 1;
  ///             d = 2;
  ///         } else {
  ///             d += 1;
  ///         }
  ///     }
  ///     n
  /// }
  prime: async func() -> stream<u64>;
  ```

There's no `match`, `loop`, `for`, `break`, `return`, indexing, tuples, strings or comments.

## Types

The closure's parameters and its result have the stream's item type, comparisons are `bool`, and `$i` is a `u64`. Operands must have the same type: there are no implicit conversions, so mixed types need `as`, e.g. `n as u64 * $i` for a `u32` `n`. A `let` bound to a literal needs its type, e.g. `let d: u64 = 2;`.

## When a stream ends

A stream ends when its next item can't be represented, rather than overflowing, as Rust's checked arithmetic would fail: on overflow, division or remainder by zero, shifting by at least the type's bits, a negative integer power, or a float result that isn't finite. So the `fibonacci` above ends at the largest Fibonacci number a `u64` holds.

A stream that never fails, e.g. `|value| !value`, is unbounded, writing items until the reader closes it. The wrapping operators never fail, except when dividing by zero, so a pseudorandom generator built from them never ends:

```wit
/// @expression || {
///     let z = seed +% ($i +% 1) *% 0x9E3779B97F4A7C15;
///     let z = (z ^ z >> 30) *% 0xBF58476D1CE4E5B9;
///     let z = (z ^ z >> 27) *% 0x94D049BB133111EB;
///     z ^ z >> 31
/// }
seeded-random: async func(seed: u64) -> stream<u64>;
```

A `while` loop whose condition never becomes `false` never ends, and nor does the stream's task, so its reader waits forever.

## Several items at once

The closure can return an array, whose elements are each an item: `[a, b]`, or `x.to_le_bytes()` and `x.to_be_bytes()` for a stream of `u8`. `$i` is then the position of the array's first item. `seeded-random-bytes` in the `random` interface writes each 64-bit number as 8 bytes, computing it once for every 8:

```wit
/// @expression || {
///     let z = seed +% ($i / 8 +% 1) *% 0x9E3779B97F4A7C15;
///     let z = (z ^ z >> 30) *% 0xBF58476D1CE4E5B9;
///     let z = (z ^ z >> 27) *% 0x94D049BB133111EB;
///     (z ^ z >> 31).to_le_bytes()
/// }
seeded-random-bytes: async func(seed: u64) -> stream<u8>;
```
