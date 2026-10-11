APP_NAME := teleprinter
VERSION := $(shell awk -F\" '/^version = / { print $$2; exit }' Cargo.toml)

# A throwaway config and data dir, so screenshots and test runs never touch
# the settings of a copy the user is running.
SCRATCH_ENV := XDG_CONFIG_HOME=$(CURDIR)/target/scratch-config XDG_DATA_HOME=$(CURDIR)/target/scratch-data

.DEFAULT_GOAL := help

.PHONY: help run dev build test check screenshot clean

help: ## Show this help
	@echo "Usage: make <target>"
	@echo
	@awk 'BEGIN { FS = ":.*## " } /^[a-z-]+:.*## / { printf "  \033[1m%-14s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

run: ## Run the app
	cargo run

dev: ## Run with live reload (requires cargo-watch)
	@command -v cargo-watch >/dev/null 2>&1 || { echo "cargo-watch is required: cargo install cargo-watch"; exit 1; }
	cargo watch -x "run --bin $(APP_NAME)"

build: ## Build a release binary
	cargo build --release

test: ## Run the tests
	cargo test

check: test ## Run the tests and clippy, as before a commit or release
	cargo clippy --all-targets -- -D warnings

screenshot: ## Regenerate docs/screenshot-*.png in a throwaway config dir
	@rm -rf target/scratch-config target/scratch-data
	@mkdir -p docs target/scratch-config target/scratch-data
	$(SCRATCH_ENV) TELEPRINTER_COMMAND=$(CURDIR)/scripts/demo.sh TELEPRINTER_SCREENSHOT=docs/screenshot-notebook.png TELEPRINTER_SKIN=Notebook cargo run
	$(SCRATCH_ENV) TELEPRINTER_COMMAND=$(CURDIR)/scripts/demo.sh TELEPRINTER_SCREENSHOT=docs/screenshot-dark-orange.png TELEPRINTER_SKIN="Dark Orange" cargo run
	$(SCRATCH_ENV) TELEPRINTER_COMMAND=$(CURDIR)/scripts/demo.sh TELEPRINTER_SCREENSHOT=docs/screenshot-split-dark-orange.png TELEPRINTER_SKIN="Dark Orange" TELEPRINTER_SPLITS=right,down cargo run
	$(SCRATCH_ENV) TELEPRINTER_COMMAND=$(CURDIR)/scripts/demo.sh TELEPRINTER_SCREENSHOT=docs/screenshot-collapsed-notebook.png TELEPRINTER_SKIN=Notebook TELEPRINTER_SPLITS=right,down,down TELEPRINTER_COLLAPSED=2,4 cargo run

clean: ## Remove build output
	cargo clean
