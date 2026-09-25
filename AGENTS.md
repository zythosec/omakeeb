# omakeeb — agent guide

A GPUI + gpui-omarchy configurator for VIA keyboards on Omarchy. Read
`README.md` for the product; this file is the working contract.

## What this is

Find a keyboard that speaks VIA, draw the layout it was given, and write each
key as it is changed. The firmware decides how many layers exist. Resetting
the keymap, clearing EEPROM and jumping to the bootloader ask first. Changing
one key does not: the keyboard stores that write immediately.

Two ways onto a machine: `make install`, and `omarchy plugin add` of this
repo (`zythosec.omakeeb`). The plugin is a bar widget and nothing else.
Omarchy's installer never runs plugin code or sudo, and the plugin does not
work around that: the button runs `packaging/open`, which opens omakeeb when
`packaging/setup --check` says the install is current, and otherwise builds
it in a floating terminal where sudo can ask for hidraw access.

## Commands

```sh
make build                      # release build
make run                        # build and run
make install                    # ~/.local, then hidraw access (asks once)
make uninstall                  # the files install put under PREFIX
make lint                       # rustfmt --check, then clippy --all-targets -D warnings
make test                       # workspace tests
make fmt                        # format in place
omarchy plugin validate .       # manifest, entry points, no symlinks
```

`make install` is the supported direct install: the release binary,
`packaging/omakeeb.desktop.in` rendered with the real prefix and the crate
version, `assets/omakeeb.svg`, and `packaging/setup --root-only`. Keep the
desktop entry's `Categories` to one main category plus additional ones, or
`desktop-file-validate` complains.

`make lint` is the gate. It must be green before anything is called done, and
it must not fix anything: a red local run is the same signal a clean check
gives.

`omarchy plugin add` clones git history. An uncommitted tree is an empty
clone, and the validator then fails for want of `manifest.json`.

## House rules

* **Strict lints.** `clippy::all` and `clippy::pedantic` are errors,
  `clippy::nursery` warns, and `-D warnings` promotes the rest. Every
  deliberate exception is listed with its reason in the workspace `Cargo.toml`
  — never silently.
* **100 columns**, 4-space indent, by `rustfmt.toml`. Only stable rustfmt
  options, because the gate runs on stable.
* **Comments say why.** The code says what. A non-obvious number, ordering or
  boundary gets the reason next to it.
* **Tests live beside the promise they make.** `omakeeb-core` tests the
  protocol frames, the keymap bounds, KLE positions, layout-option bits and
  the definition catalog. The window has no harness yet; do not claim a
  screen was exercised because the binary compiled.
* **Never write a key outside the matrix the definition declared.**
  `Keymap::set` refuses it, and that refusal is tested.
* **Do not add or remove layers.** The count comes from the keyboard. Shifting
  layers in EEPROM to invent a slot discards keys the firmware still has.

## Invariants

1. **Open the keyboard through hidraw, never libusb.** Libusb detaches the
   kernel driver and the board stops typing. VIA is usage page `0xFF60`,
   usage `0x61`. A report is 32 bytes. Multi-byte integers are big-endian.
2. **The layer count is the firmware's.** `get_layer_count` is that number.
   The window shows all of them. There is no command that grows the EEPROM.
3. **A definition is required before a keymap is read or written.** The
   protocol does not say how many rows and columns there are. Guessing a
   matrix and writing it addresses the wrong EEPROM.
4. **A provided definition is remembered under the keyboard's USB ids.**
   `catalog::remember` stamps `vendorId` and `productId` and writes
   `~/.config/omakeeb/definitions/VVVV-PPPP.json`. The next plug-in finds
   that file and does not ask again. A second JSON with the same ids does
   not hide the one we wrote.
5. **KLE positions follow the serialized format.** A property object applies
   to the next key, width and height reset after that key, and a new row
   moves down one unit and back to the left. Layout-option bits are packed
   from the least significant bit, one group at a time.
6. **Keymap addresses are layer, then row, then column, two bytes each.**
   `keymap_offset` is that formula. Bulk reads use the 28-byte frame; a
   firmware that rejects the buffer is read one key at a time.
7. **Lighting changes are followed by `custom_save`.** A keymap write persists
   on its own. A lighting write does not.
8. **Macros are written as one buffer, guarded.** Every macro shares it, so a
   save writes them all. The buffer's last byte goes non-zero first and back
   to zero last, because QMK will not play a macro while it is set. Bytes
   omakeeb does not edit are kept as they were read.
9. **Chrome is on the rem scale. The board is in key units.** Spacing and type
   live in `ui.rs`. The keyboard is scaled from those units into pixels
   because the geometry comes from the layout, not from the type. Corners
   stay square.
10. **The plugin folder has no symlinks.** `omarchy plugin validate` rejects
    them. `manifest.json` stays at the repo root, with `BarWidget.qml` beside
    it. The plugin has no service: nothing builds or asks for a password
    until the user presses the button.

## Where changes belong

| change | where |
| --- | --- |
| a VIA frame, or its byte order | `crates/omakeeb-core/src/protocol.rs` |
| what may be stored, and where | `crates/omakeeb-core/src/keymap.rs` |
| a keyboard definition, KLE, layout options | `crates/omakeeb-core/src/layout.rs` |
| remembering a definition on disk | `crates/omakeeb-core/src/catalog.rs` |
| names a person can type, and the picker groups | `crates/omakeeb-core/src/keycode.rs` |
| lighting channels | `crates/omakeeb-core/src/lighting.rs` |
| the macro buffer, and the `{…}` text a person edits | `crates/omakeeb-core/src/macros.rs` |
| finding or opening a hidraw node | `crates/omakeeb-core/src/hid.rs` |
| a command the live keyboard must answer | `crates/omakeeb-core/src/session.rs` |
| the in-memory keyboard used by tests and `--demo` | `crates/omakeeb-core/src/transport.rs` |
| a key, a screen, a confirmation | `crates/omakeeb-app/src/app.rs` |
| spacing, type and size | `crates/omakeeb-app/src/ui.rs` — tokens only |
| drawing the keys | `crates/omakeeb-app/src/board.rs` |
| building, installing, granting hidraw | `packaging/setup`, `Makefile` |
| removing all of it, plugin included | `packaging/uninstall` |
| the bar button, and build-on-first-press | `BarWidget.qml`, `packaging/open`, `manifest.json` |

## Verification expectations

* Protocol framing, keymap bounds, KLE geometry, layout-option packing,
  definition loading, the macro buffer and its text syntax, and a scripted
  keyboard session are covered by `omakeeb-core` tests.
* `make lint` is clean.
* The window was run against a wired VIA keyboard. It has not been checked
  in every theme, and there is no window-harness test yet.
