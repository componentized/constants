# `factory`

Create components whose exported functions return constant values, from a WIT world with values in its doc comments.

## Interfaces

Imports:

- `componentized:component/types@0.0.0-dev`: the `component` and `error` types, only types are imported
- `componentized:component/wit@0.0.0-dev`: the `wit` type for parsed WIT, only types are imported

Exports:

- `componentized:constants/factory@0.1.2-dev`: creates a component implementing a world from its WIT
