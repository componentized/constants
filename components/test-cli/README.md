# `test-cli`

A command calling the functions of the constants data interfaces, for testing: [`test-cli-core`](../test-cli-core), composed with the `math`, `atomics`, `random` and `types` components implementing the interfaces it imports. It only imports WASI, so it runs with `wasmtime run`:

```sh
make components/test-cli
wasmtime run target/components/test-cli/test-cli.wasm math pi

# will continue running until stdin closes
printf '\n\n\n' | wasmtime run target/components/test-cli/test-cli.wasm atomics incrementer 10

# a different seed will produce a different set of numbers
printf '\n\n\n' | wasmtime run target/components/test-cli/test-cli.wasm random seeded-random 42
```

See [`test-cli-core`](../test-cli-core) for its arguments and how it reads streams.
