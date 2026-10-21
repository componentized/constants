# This repository builds with its own build, its components and cli exist to smoke test it, see
# README.md

# the cli, released with an archive for each platform, and published to crates.io
CLI_BIN := constants
PUBLISH_CRATES := componentized-constants componentized-constants-cli

# the cli installed as a tool built from the workspace, like a cli the components are built with, e.g.
# the constants cli in componentized/constants, run as `$(HELLO)`
LOCAL_TOOLS := componentized-constants-cli

include build/build.mk

test: test-grammar
test-integration: cmd = math pi
test-integration: run

.PHONY: run ## Run the test-cli to get a constant value, e.g. `make run cmd="math pi"`
run: ${COMPONENTS_DIR}/test-cli/test-cli.debug.wasm | $(call tool,wasmtime-cli)
	@$(WASMTIME) run -Wcomponent-model-map ${COMPONENTS_DIR}/test-cli/test-cli.debug.wasm $(cmd)

# the tree-sitter grammar for `@expression` closures, see grammars/constants-expression/README.md. The parser is generated
# with tree-sitter's own JavaScript runtime rather than node, then built and tested against its corpus.
.PHONY: test-grammar ## Test the tree-sitter grammar for expressions
test-grammar: | $(call tool,tree-sitter-cli)
	cd grammars/constants-expression && \
		TREE_SITTER_JS_RUNTIME=native $(TREE_SITTER) generate && \
		$(TREE_SITTER) test

.PHONY: install ## install constants cli to CARGO_INSTALL_ROOT
install:
	cargo +$(RUST_TOOLCHAIN) install --path . --locked
