# Constants <!-- omit in toc -->

Create custom wasm components that export constant values.

WIT defines the shape of a component, but it can't define values, even ones that never change. Constants fills that gap: give it a WIT world whose exported functions take no parameters, with each function's value written in its doc comment, and it generates a component whose functions return those values.

- [Usage](#usage)
  - [Overrides](#overrides)
  - [Supported types](#supported-types)
  - [Values](#values)
- [Factory component](#factory-component)
- [Build](#build)
- [Community](#community)
  - [Code of Conduct](#code-of-conduct)
  - [Communication](#communication)
  - [Contributing](#contributing)
- [License](#license)

## Usage

Define the world to implement. Each function's value is a [WAVE](https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wasm-wave) expression following a `@value` tag in its doc comment:

```wit
package example:constants;

interface types {
    record point {
        x: s32,
        y: s32,
    }

    type port = u16;
}

interface constants {
    use types.{point, port};

    /// @value "hello"
    greeting: func() -> string;

    /// The port to serve HTTP traffic on.
    ///
    /// @value 8080
    http-port: func() -> port;

    /// @value {x: 0, y: 0}
    origin: func() -> point;

    /// @value [
    ///   ("http", 80),
    ///   ("https", 443),
    /// ]
    ports: func() -> list<tuple<string, port>>;
}

world config {
    /// @value "1.0.0"
    export version: func() -> string;
    export constants;
}
```

Generate the component:

```sh
constants --wit ./wit --world config -o constants.wasm
```

The `@value` tag must start a line of the doc comment. Everything after the tag, including any following lines, is the value, so put any description before it. Doc comments aren't carried into the generated component's WIT, so the values don't appear there.

The generated component imports `example:constants/types`, the interface that defines the `point` and `port` types. It imports only types, never functions, so any host or composition that supplies these types can satisfy it.

### Overrides

A WAVE file passed with `--overrides` replaces values from the WIT, e.g. to build variants of the same component for different environments. It holds a single record with a field per world export:

- a function export's field holds that function's value
- an interface export's field is named after the interface and holds a nested record with a field per function

Every field is optional; anything omitted keeps its `@value` from the WIT:

```
// production.wave
{
  version: "1.0.1",
  constants: {
    http-port: 80,
  },
}
```

```sh
constants --wit ./wit --world config --overrides production.wave -o constants.wasm
```

Every exported function needs a value from either its `@value` tag or an override.

### Supported types

Every exported function must be synchronous, take no parameters, and return a value. Values can be expressed for:

- `bool`, `s8`, `s16`, `s32`, `s64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`, `char` and `string`
- `list<T>` and fixed-length `list<T, N>`
- `tuple<...>`
- `record`
- `flags`
- `enum`
- `variant`
- `option`
- `result`

and type aliases of any of these.

Values can't be expressed for `own`, `borrow`, `stream`, `future` or `error-context`. Result types may still include them, as long as the value doesn't reach them: an `option<own<file>>` can be `none`, a `list<future<u8>>` can be `[]`, a `result<u8, stream<u8>>` can be `ok(1)`, and a variant with a case whose payload is one of these types can use any other case. A value that does reach one is an error.

`map` isn't supported until WAVE defines a syntax for map values.

### Values

Values, whether in `@value` tags or overrides, are written in WAVE. `//` line comments are allowed.

Record fields can appear in any order. Every field is required, except fields with an `option` type, which default to `none` when omitted.

Options are written as `some(value)` or `none`. The `some(...)` may be left off, e.g. `7` rather than `some(7)`, unless the payload is itself an option or result: `option<option<u8>>` values must be written as `none`, `some(none)` or `some(some(7))`.

Results are written as `ok(value)` or `err(value)`, or just `ok` or `err` for a case without a payload, e.g. `result<_, string>` values are written as `ok` or `err("message")`. The `ok(...)` may be left off, e.g. `7` rather than `ok(7)`, unless the payload is an option or result.

Enum values are written as the case name, e.g. `info`. Variant values are written as the case name, followed by the payload in parentheses when the case has one, e.g. `circle(1.5)` or `empty`. Case names that are WAVE keywords (`true`, `false`, `some`, `none`, `ok`, `err`, `inf` and `nan`) must be escaped with `%`, e.g. `%none`.

## Factory component

The factory is a wasm component that creates constants components, for hosts and compositions that build them at runtime. It exports [`componentized:constants/factory`](./wit/factory.wit), which uses types from [`componentized:component`](https://github.com/componentized/component): `component` and `error` from its `types` interface, and `wit` from its `wit` interface:

```wit
variant wit-source {
    text(string),
    encoded(component),
    parsed(wit),
}

variant overrides {
    wave(string),
}

create: async func(
    wit: wit-source,
    %world: option<string>,
    overrides: option<overrides>,
) -> result<component, error>;
```

The WIT can be given as `text`, with any packages the world depends on defined inline in nested `package ns:name { ... }` blocks. It can also be given as `encoded`, a WIT package encoded as wasm, e.g. from `wasm-tools component wit --wasm` or `wkg build`, which carries its dependencies with it. Or it can be given as `parsed`, the `wit` record returned by `componentized:component/wit#extract`, e.g. from the [`extract-wit`](https://github.com/componentized/component/tree/main/components/extract-wit) component. `world` works the same as the CLI's `--world` flag, and `overrides` given as `wave(...)` holds the same WAVE as an `--overrides` file. `overrides` is a variant so other ways to give values can be added later.

`extract` also accepts components, and so can the factory: when the `parsed` WIT was extracted from a component rather than a WIT package, the factory implements that component's world unless `world` names another one. Components don't carry doc comments, so there are no `@value` tags; every value must come from `overrides`.

The factory imports `componentized:component/types` and `componentized:component/wit`, but only for their types, not their functions, so hosts can satisfy them with empty instances. Hosts must support the component model async ABI and maps; with `wasmtime run`, enable them with `-W component-model-async=y,component-model-map=y`:

```sh
wasmtime run -W component-model-async=y,component-model-map=y \
  --invoke 'create(text("package a:b; world w { /// @value 42\n export answer: func() -> u32; }"), none, none)' \
  lib/factory.wasm
```

## Build

Prereqs:
- a rust toolchain
- [`wasm-tools`](https://github.com/bytecodealliance/wasm-tools)
- [`wkg`](https://github.com/bytecodealliance/wasm-pkg-tools)

```sh
make components
```

The build creates each component in [`components`](./components) into `lib`, e.g. the factory at `lib/factory.wasm`, along with `lib/interface.wasm`, the `componentized:constants` WIT package. Each component is also built with debug info, e.g. `lib/factory.debug.wasm`.

To run the tests, which exercise the CLI and the components, using the `extract-wit` component from `componentized:component`, which the build fetches to `lib/dep-extract-wit.wasm`:

```sh
make test
```

The WIT dependencies in each `wit/deps` directory are fetched rather than committed, pinned by the `wkg.lock` files. The make targets fetch them as needed. To fetch or update them directly, e.g. before building the Rust components with `cargo`, whose bindings are generated from the WIT:

```sh
make wit
```

## Community

### Code of Conduct

The Componentized project follow the [Contributor Covenant Code of Conduct](./CODE_OF_CONDUCT.md). In short, be kind and treat others with respect.

### Communication

General discussion and questions about the project can occur in the project's [GitHub discussions](https://github.com/orgs/componentized/discussions).

### Contributing

The Componentized project team welcomes contributions from the community. A contributor license agreement (CLA) is not required. You own full rights to your contribution and agree to license the work to the community under the Apache License v2.0, via a [Developer Certificate of Origin (DCO)](https://developercertificate.org). For more detailed information, refer to [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache License v2.0: see [LICENSE](./LICENSE) for details.
