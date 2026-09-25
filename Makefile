# omakeeb: build, check, and install.
#
# `make install` puts the binary, a desktop entry and an icon under PREFIX
# (default: ~/.local), so omakeeb shows up in the Omarchy launcher. The
# default needs no root.

PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
APPDIR ?= $(PREFIX)/share/applications
ICONDIR ?= $(PREFIX)/share/icons/hicolor/scalable/apps
LIBDIR ?= $(PREFIX)/lib/omakeeb

MANIFEST = Cargo.toml
CARGO ?= cargo
TARGET = target/release/omakeeb
ICON = assets/omakeeb.svg
DESKTOP = packaging/omakeeb.desktop.in

.PHONY: help build run install uninstall rules lint test fmt clean

help:
	@echo "omakeeb"
	@echo
	@echo "  make build       release build"
	@echo "  make run         build and run"
	@echo "  make install     install to $(PREFIX): binary, desktop entry, icon"
	@echo "  make uninstall   remove what install put there"
	@echo "  make rules       print the udev install steps"
	@echo "  make lint        rustfmt --check and clippy -D warnings"
	@echo "  make test        core tests"
	@echo "  make fmt         format in place"
	@echo "  make clean       cargo clean"

build:
	$(CARGO) build --release

run: build
	$(TARGET)

lint:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --all-targets -- -D warnings

test:
	$(CARGO) test --workspace

fmt:
	$(CARGO) fmt --all

install: build
	install -d $(BINDIR) $(APPDIR) $(ICONDIR) $(LIBDIR)
	install -m755 $(TARGET) $(BINDIR)/omakeeb
	install -m644 $(ICON) $(ICONDIR)/omakeeb.svg
	install -m755 packaging/setup packaging/uninstall packaging/omakeeb-hid $(LIBDIR)/
	install -m644 packaging/50-omakeeb.rules $(LIBDIR)/
	packaging/setup --root-only
	VERSION=$$(sed -n 's/^version = "\(.*\)"/\1/p' $(MANIFEST) | head -1) && \
	sed -e 's|@EXEC@|$(BINDIR)/omakeeb|' -e "s|@VERSION@|$$VERSION|" \
	    $(DESKTOP) > $(APPDIR)/omakeeb.desktop && \
	chmod 644 $(APPDIR)/omakeeb.desktop
	@if command -v update-desktop-database >/dev/null 2>&1; then \
	    update-desktop-database $(APPDIR) 2>/dev/null || true; \
	fi
	@echo
	@echo "installed:"
	@echo "  $(BINDIR)/omakeeb"
	@echo "  $(APPDIR)/omakeeb.desktop"
	@echo "  $(ICONDIR)/omakeeb.svg"
	@echo "  /usr/lib/udev/omakeeb-hid"
	@echo "  /etc/udev/rules.d/50-omakeeb.rules"
	@case ":$$PATH:" in *":$(BINDIR):"*) ;; *) \
	    echo; echo "note: $(BINDIR) is not on PATH in this shell";; esac

uninstall:
	rm -f $(BINDIR)/omakeeb $(APPDIR)/omakeeb.desktop $(ICONDIR)/omakeeb.svg
	rm -rf $(LIBDIR)
	@if command -v update-desktop-database >/dev/null 2>&1; then \
	    update-desktop-database $(APPDIR) 2>/dev/null || true; \
	fi
	@echo "removed"

rules:
	$(CARGO) run -q -p omakeeb-app -- --rules

clean:
	$(CARGO) clean
