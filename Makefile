APP_NAME := teleprinter
APP_ID := com.sectorflabs.Teleprinter
PREFIX ?= $(HOME)/.local
BINDIR := $(PREFIX)/bin
APPSDIR := $(PREFIX)/share/applications
ICONDIR := $(PREFIX)/share/icons/hicolor
VERSION := $(shell awk -F\" '/^version = / { print $$2; exit }' Cargo.toml)

# A throwaway config and data dir, so screenshots and test runs never touch
# the settings of a copy the user is running.
SCRATCH_ENV := XDG_CONFIG_HOME=$(CURDIR)/target/scratch-config XDG_DATA_HOME=$(CURDIR)/target/scratch-data

.DEFAULT_GOAL := help

.PHONY: help run dev build test check screenshot icons install uninstall clean icon

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

icons: ## Render assets/teleprinter-icon-128.png from packaging/linux/teleprinter.svg
	cargo run -q --example render_icon -- 128 assets/teleprinter-icon-128.png

icon: ## docs/icon.png: the app icon at 120 px, for the README
	scripts/icon.sh

# For the current user; no sudo. The launcher's Exec gets the absolute path,
# so it works even if $(BINDIR) isn't on the GNOME session's PATH. On
# Wayland the window icon comes from this launcher, matched by app id.
install: build ## Install Teleprinter and its launcher for the current user (PREFIX=~/.local)
	install -Dm755 target/release/teleprinter $(BINDIR)/teleprinter
	install -Dm644 packaging/linux/teleprinter.svg $(ICONDIR)/scalable/apps/$(APP_ID).svg
	install -Dm644 assets/teleprinter-icon-128.png $(ICONDIR)/128x128/apps/$(APP_ID).png
	install -d $(APPSDIR)
	sed 's|^Exec=teleprinter|Exec=$(BINDIR)/teleprinter|' packaging/linux/$(APP_ID).desktop > $(APPSDIR)/$(APP_ID).desktop
	-update-desktop-database -q $(APPSDIR)
	@# GTK trusts an icon cache over the folder, so refresh one if it exists.
	-@test ! -f $(ICONDIR)/icon-theme.cache || gtk-update-icon-cache -q -t -f $(ICONDIR)
	@echo "Installed $(BINDIR)/teleprinter and the $(APP_ID) launcher"

uninstall: ## Remove what make install put in place
	rm -f $(BINDIR)/teleprinter $(APPSDIR)/$(APP_ID).desktop
	rm -f $(ICONDIR)/scalable/apps/$(APP_ID).svg $(ICONDIR)/128x128/apps/$(APP_ID).png
	-update-desktop-database -q $(APPSDIR)
	-@test ! -f $(ICONDIR)/icon-theme.cache || gtk-update-icon-cache -q -t -f $(ICONDIR)

clean: ## Remove build output
	cargo clean
