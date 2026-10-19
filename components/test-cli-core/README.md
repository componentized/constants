# `test-cli-core`

A command calling the functions of the constants data interfaces, for testing.

```sh
test-cli <interface> <function> [arguments...]
```

The arguments are parsed as the function's parameters, and its result is printed. A stream's first item is printed, then items are read as stdin's lines ask for them: an empty line reads the next item, a number reads that many. The command ends when the stream ends or stdin closes. When stdin is a terminal, whose echo of each line's newline already ends the line, an item isn't followed by a newline of its own, so items aren't separated by blank lines. Without arguments, it lists the functions it can call.

It imports `math`, `atomics`, `random` and the numeric type interfaces, `u8` to `f64`. [`test-cli`](../test-cli) composes it with the components implementing them, so it runs with `wasmtime run`:

```sh
make components/test-cli
wasmtime run target/components/test-cli/test-cli.wasm math pi
printf '\n\n3\n' | wasmtime run target/components/test-cli/test-cli.wasm atomics incrementer 10
```

Its dispatch is generated from the `test-cli-core` world in [`components/wit/worlds.wit`](../wit/worlds.wit) by [`build.rs`](./build.rs), so it calls each function of each imported interface.

It exports WASIp3's `wasi:cli/run@0.3.0`, whose `run` is async, so the command can wait for a stream's items, which a synchronous `run` can't, the component model traps instead. It imports the other WASIp3 `wasi:cli` interfaces it needs: `environment` for its arguments, `stdin`, read from a stream, and `stdout` and `stderr`, written to streams. It's built like the other rust components, for `wasm32-unknown-unknown`.
