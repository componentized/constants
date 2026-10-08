# `factory`

Create components whose exported functions return constant values, from a WIT world with values in its doc comments.

## Interfaces

Imports:

- `componentized:component/types@0.1.0`: the `component` and `error` types, only types are imported
- `componentized:component/wit@0.1.0`: the `wit` type for parsed WIT, only types are imported

Exports:

- `componentized:constants/factory@0.2.0-dev`: creates a component implementing a world from its WIT
