SHELL := /bin/bash

WKG_CONFIG := $(CURDIR)/.config/wasm-pkg/config.toml

export RUST_BACKTRACE ?= 1
export WASMTIME_BACKTRACE_DETAILS ?= 1

COMPONENTS_DIR := $(abspath target/components)
TOOLS_DIR := $(abspath target/tools)
export PATH := $(TOOLS_DIR)/bin:$(PATH)

# the toolchain pinned in rust-toolchain.toml, named for `cargo install`, otherwise rustup warns that
# the toolchain file overrides the default
RUST_TOOLCHAIN := $(shell sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)

# cargo binstall downloads prebuilt binaries, without it the tools are built with cargo install
CARGO_INSTALL := $(if $(shell command -v cargo-binstall 2> /dev/null),cargo binstall --no-confirm --disable-telemetry,cargo install)

COMPONENTS = $(sort $(foreach file,$(wildcard $(addprefix components/*/,wit/*.constants.wit *.properties *.wac *.wkg Cargo.toml)),$(word 2,$(subst /, ,$(file)))))
TOOLS := static-config wac-cli wasm-opt wasm-tools wkg

# the tools are run by path, make runs simple commands itself rather than with a shell, finding them
# on the PATH make was started with, not the PATH exported above
CONSTANTS := $(TOOLS_DIR)/bin/constants
STATIC_CONFIG := $(TOOLS_DIR)/bin/static-config
WAC := $(TOOLS_DIR)/bin/wac
WASM_OPT := $(TOOLS_DIR)/bin/wasm-opt
WASM_TOOLS := $(TOOLS_DIR)/bin/wasm-tools
WKG := $(TOOLS_DIR)/bin/wkg

# the wasm features rust enables for wasm32-unknown-unknown, `rustc --print cfg --target
# wasm32-unknown-unknown`. The stripped modules don't say which features they use, wasm-opt is
# limited to these so it doesn't introduce instructions rust wouldn't emit.
WASM_OPT_FEATURES := --enable-bulk-memory --enable-multivalue --enable-mutable-globals --enable-nontrapping-float-to-int --enable-reference-types --enable-sign-ext


.PHONY: all
all: build components

.PHONY: build
build:
	cargo build --release

.PHONY: install
install:
	cargo +$(RUST_TOOLCHAIN) install --path . --locked

.PHONY: clean
clean: clean-wit
	cargo clean

.PHONY: clean-components
clean-components: clean-wit
	rm -rf ${COMPONENTS_DIR}

.PHONY: clean-wit ## Remove the fetched wit dependencies, fetched again by `make wit`
clean-wit:
	rm -rf wit/deps components/wit/deps components/*/wit/deps

.PHONY: test
test: components
	cargo test --workspace


tool_version = $(shell sed -n 's/^$(1) = "=\(.*\)"$$/\1/p' tools/Cargo.toml)
# a stamp naming the version of a tool installed in target/tools/bin, e.g. `wkg@0.16.1`, the binary
# does not say which version it is. Bumping the pinned version names a stamp that does not exist yet,
# so the tool is installed again.
tool = $(TOOLS_DIR)/.installed/$(1)@$(call tool_version,$(1))

.PHONY: tools ## Install the cli tools pinned in tools/Cargo.toml, and the constants cli from source
tools: $(foreach name,$(TOOLS),$(call tool,$(name))) $(CONSTANTS)

define INSTALL_TOOL

$(call tool,$1):
	$(CARGO_INSTALL) --locked --root $(TOOLS_DIR) --version $(call tool_version,$1) $1
	@mkdir -p $$(@D)
	@# only the installed version has a stamp, so going back to a previous version installs it again
	@rm -f $$(@D)/$1@*
	@touch $$@

endef

$(foreach name,$(TOOLS),$(eval $(call INSTALL_TOOL,$(name))))

$(CONSTANTS): Cargo.toml Cargo.lock rust-toolchain.toml $(shell find src crates/componentized-constants -type f)
	@# forced, make only installs when the sources change, and the binary may belong to another package
	cargo +$(RUST_TOOLCHAIN) install --force --locked --root $(TOOLS_DIR) --path .
	@# cargo leaves the binary alone when it is already up to date
	@touch $@

.PHONY: components
components: ${COMPONENTS_DIR}/interface.wasm $(foreach component,$(COMPONENTS),${COMPONENTS_DIR}/$(component)/$(component).wasm ${COMPONENTS_DIR}/$(component)/$(component).debug.wasm)

define BUILD_COMPONENT

.PHONY: components/$1
components/$1: ${COMPONENTS_DIR}/$1/$1.wasm ${COMPONENTS_DIR}/$1/$1.debug.wasm

ifneq ($(wildcard components/$1/wit/$1.constants.wit),)

${COMPONENTS_DIR}/$1/$1.wasm: components/$1/wit/deps ${COMPONENTS_DIR}/$1/README.md $(CONSTANTS)
	$(CONSTANTS) --wit components/$1/wit -o ${COMPONENTS_DIR}/$1/$1.wasm

${COMPONENTS_DIR}/$1/$1.debug.wasm: components/$1/wit/deps ${COMPONENTS_DIR}/$1/README.md $(CONSTANTS)
	$(CONSTANTS) --wit components/$1/wit -o ${COMPONENTS_DIR}/$1/$1.debug.wasm

components/$1/wit/deps: wit/deps components/$1/wkg.toml $(shell find components/$1/wit -type f -name "*.wit" -not -path "*/deps/*" 2>/dev/null) | $(call tool,wkg)
	( cd components/$1 ; $(WKG) fetch --config $(WKG_CONFIG) )

else ifneq ($(wildcard components/$1/$1.properties),)

${COMPONENTS_DIR}/$1/$1.wasm: components/$1/$1.properties ${COMPONENTS_DIR}/$1/README.md | $(call tool,static-config)
	$(STATIC_CONFIG) -f components/$1/$1.properties -o ${COMPONENTS_DIR}/$1/$1.wasm

${COMPONENTS_DIR}/$1/$1.debug.wasm: components/$1/$1.properties ${COMPONENTS_DIR}/$1/README.md | $(call tool,static-config)
	$(STATIC_CONFIG) -f components/$1/$1.properties -o ${COMPONENTS_DIR}/$1/$1.debug.wasm

else ifneq ($(wildcard components/$1/$1.wac),)

# the local packages the composition instantiates, e.g. `new local:latch-n2 { ... }`
WAC_DEPS_$1 := $$(shell grep -v '^\s*//' components/$1/$1.wac | grep -oE 'local:[a-z0-9-]+' | sed 's/^local://' | sort -u)

${COMPONENTS_DIR}/$1/$1.wasm: components/$1/$1.wac $$(foreach component,$$(WAC_DEPS_$1),$${COMPONENTS_DIR}/$$(component)/$$(component).wasm) ${COMPONENTS_DIR}/$1/README.md | $(call tool,wac-cli)
	$(WAC) compose $$(foreach component,$$(WAC_DEPS_$1),-d local:$$(component)=$${COMPONENTS_DIR}/$$(component)/$$(component).wasm) -o ${COMPONENTS_DIR}/$1/$1.wasm components/$1/$1.wac

${COMPONENTS_DIR}/$1/$1.debug.wasm: components/$1/$1.wac $$(foreach component,$$(WAC_DEPS_$1),$${COMPONENTS_DIR}/$$(component)/$$(component).debug.wasm) ${COMPONENTS_DIR}/$1/README.md | $(call tool,wac-cli)
	$(WAC) compose $$(foreach component,$$(WAC_DEPS_$1),-d local:$$(component)=$${COMPONENTS_DIR}/$$(component)/$$(component).debug.wasm) -o ${COMPONENTS_DIR}/$1/$1.debug.wasm components/$1/$1.wac

else ifneq ($(wildcard components/$1/$1.wkg),)

${COMPONENTS_DIR}/$1/$1.wasm: components/$1/$1.wkg ${COMPONENTS_DIR}/$1/README.md | $(call tool,wkg)
	$(WKG) oci pull $(shell cat components/$1/$1.wkg 2> /dev/null | head -1) -o ${COMPONENTS_DIR}/$1/$1.wasm

${COMPONENTS_DIR}/$1/$1.debug.wasm: components/$1/$1.wkg ${COMPONENTS_DIR}/$1/README.md | $(call tool,wkg)
	$(WKG) oci pull $(shell cat components/$1/$1.wkg  2> /dev/null | tail -1 2> /dev/null) -o ${COMPONENTS_DIR}/$1/$1.debug.wasm

# cargo is checked last, other strategies may have a Cargo.toml for tests of non-rust sources
else ifneq ($(wildcard components/$1/Cargo.toml),)

${COMPONENTS_DIR}/$1/$1.wasm: Cargo.toml Cargo.lock components/wit/deps $(shell find components/$1 -type f) $(shell find crates -type f) ${COMPONENTS_DIR}/$1/README.md | $(call tool,wasm-opt) $(call tool,wasm-tools)
	cargo build -p $1 --target wasm32-unknown-unknown --profile component
	$(WASM_OPT) -Oz $(WASM_OPT_FEATURES) target/wasm32-unknown-unknown/component/$(subst -,_,$1).wasm -o target/wasm32-unknown-unknown/component/$(subst -,_,$1).opt.wasm
	$(WASM_TOOLS) component new target/wasm32-unknown-unknown/component/$(subst -,_,$1).opt.wasm -o ${COMPONENTS_DIR}/$1/$1.wasm

${COMPONENTS_DIR}/$1/$1.debug.wasm: Cargo.toml Cargo.lock components/wit/deps $(shell find components/$1 -type f) $(shell find crates -type f) ${COMPONENTS_DIR}/$1/README.md | $(call tool,wasm-tools)
	cargo build --target wasm32-unknown-unknown -p $1
	$(WASM_TOOLS) component new target/wasm32-unknown-unknown/debug/$(subst -,_,$1).wasm -o ${COMPONENTS_DIR}/$1/$1.debug.wasm

endif

${COMPONENTS_DIR}/$1/README.md: components/$1/README.md
	@mkdir -p ${COMPONENTS_DIR}/$1
	@cp components/$1/README.md ${COMPONENTS_DIR}/$1/README.md

endef

$(foreach component,$(COMPONENTS),$(eval $(call BUILD_COMPONENT,$(component))))

${COMPONENTS_DIR}/interface.wasm: wit/deps README.md | $(call tool,wkg)
	@mkdir -p ${COMPONENTS_DIR}
	$(WKG) build --config $(WKG_CONFIG) -o ${COMPONENTS_DIR}/interface.wasm
	@cp README.md ${COMPONENTS_DIR}/README.md

.PHONY: wit
wit: wit/deps components/wit/deps

# the wit dependencies are fetched rather than committed, see .gitignore
wit/deps: wkg.toml $(shell find wit -type f -name "*.wit" -not -path "*/deps/*") | $(call tool,wkg)
	$(WKG) fetch --config $(WKG_CONFIG)

components/wit/deps: wit/deps components/wkg.toml $(shell find components/wit -type f -name "*.wit" -not -path "*/deps/*") | $(call tool,wkg)
	( cd components && $(WKG) fetch --config $(WKG_CONFIG) )

# sign published components with cosign, `SIGN=false` to push without signing, e.g. to a local registry
SIGN ?= true
# append each published file and its image to this file, e.g. `factory.wasm ghcr.io/componentized/constants/factory:0.1.0@sha256:...`
PUBLISH_LOG ?=

# the files that can be published, e.g. gate.wasm, published from target/components/gate/gate.wasm
PUBLISH_FILES := interface.wasm $(foreach component,$(filter-out dep-% test-%,$(COMPONENTS)),$(component).wasm $(component).debug.wasm)

.PHONY: publish ## Publish each component in the target/components directory
publish: $(addprefix publish-,$(PUBLISH_FILES))

.PHONY: $(addprefix publish-,$(PUBLISH_FILES))
$(addprefix publish-,$(PUBLISH_FILES)): publish-%: | $(call tool,wkg)
	@VERSION="$(VERSION)" REPOSITORY="$(REPOSITORY)" COMPONENTS_DIR="$(COMPONENTS_DIR)" SIGN="$(SIGN)" PUBLISH_LOG="$(PUBLISH_LOG)" \
		scripts/publish.sh $*
