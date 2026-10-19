SHELL := /bin/bash

export RUST_BACKTRACE ?= 1
export WASMTIME_BACKTRACE_DETAILS ?= 1
export WKG_CONFIG_FILE := $(abspath .config/wasm-pkg/config.toml)

# `make` without a goal runs all, otherwise the first target of an included file would run
.DEFAULT_GOAL := all

# tools first, the others use the variables it defines for each tool when their rules are read
include make/tools.mk

# the constants cli is built from this repository rather than pinned in tools/Cargo.toml, installed
# with the pinned tools, the components are built with it
CONSTANTS := $(TOOLS_BIN)/constants

include make/wit.mk
include make/components.mk
include make/publish.mk

# the toolchain pinned in rust-toolchain.toml, named for `cargo install`, otherwise rustup warns that
# the toolchain file overrides the default
RUST_TOOLCHAIN := $(shell sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)


.PHONY: all
all: tools wit components test

.PHONY: build
build:
	cargo build --release

.PHONY: install
install:
	cargo +$(RUST_TOOLCHAIN) install --path . --locked

.PHONY: clean
clean: clean-components clean-wit
	@:

.PHONY: clean-all
clean-all: clean-components clean-tools clean-wit
	cargo clean

.PHONY: test
test: components test-grammar
	cargo test --workspace

.PHONY: run ## Run the test-cli to get a constant value, e.g. `make run cmd="math pi"`
run: ${COMPONENTS_DIR}/test-cli/test-cli.debug.wasm | $(call tool,wasmtime-cli)
	@$(WASMTIME) run -Wcomponent-model-map ${COMPONENTS_DIR}/test-cli/test-cli.debug.wasm $(cmd)

tools: $(CONSTANTS)

$(CONSTANTS): Cargo.toml Cargo.lock rust-toolchain.toml $(shell find src crates/componentized-constants -type f)
	@# forced, make only installs when the sources change, and the binary may belong to another package
	cargo +$(RUST_TOOLCHAIN) install --force --locked --root $(TOOLS_DIR) --path .
	@# cargo leaves the binary alone when it is already up to date
	@touch $@

# the tree-sitter grammar for `@expression` closures, see grammars/constants-expression/README.md. The parser is generated
# with tree-sitter's own JavaScript runtime rather than node, then built and tested against its corpus.
.PHONY: test-grammar ## Test the tree-sitter grammar for expressions
test-grammar: | $(call tool,tree-sitter-cli)
	cd grammars/constants-expression && \
		TREE_SITTER_JS_RUNTIME=native $(TREE_SITTER) generate && \
		$(TREE_SITTER) test
