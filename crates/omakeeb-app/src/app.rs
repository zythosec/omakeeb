//! Window state. Marking a key writes it immediately. Resetting the keymap,
//! clearing EEPROM and jumping to the bootloader ask first, because those
//! cannot be taken back one key at a time.

use std::path::{Path, PathBuf};

use gpui_kit::{
    Context, FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
    relative, svg,
};
use gpui_omarchy::{
    ActiveTheme, ButtonVariant, IconName, Status, alert, badge, button, empty_state, icon,
    icon_button, keycap, separator, with_tooltip,
};
use omakeeb_core::{
    Catalog, Definition, Discovered, Error, Group, HidLink, Identity, Item, Session,
    definitions_dir, demo_session, discover, effect_label, items_for, long_name, macro_index,
    macros, parse_keycode, remember, short_name,
};

use crate::board;
use crate::ui::{self, size, space, text};

/// Printed by `--rules`. Access itself is granted by `make install`.
pub const UDEV_HELP: &str = "\
make install grants the active seat access to VIA keyboards.
It asks for an administrator password once, installs /usr/lib/udev/omakeeb-hid,
and reloads udev. After that, plugging a keyboard in is enough.
";

/// What the command line asked for.
#[derive(Clone, Debug)]
pub struct Launch {
    pub demo: bool,
    pub rules: bool,
    pub definition: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// No keyboard, or more than one, or hidraw refused the open.
    Devices,
    /// The keyboard answered, and no saved layout matches its USB ids.
    NeedLayout,
    Board,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overlay {
    None,
    Help,
    Confirm(Confirm),
    /// The text lives in `Omakeeb::macro_edit`, which is set while this is.
    Macro,
}

/// A macro being edited. Nothing reaches the keyboard until enter.
#[derive(Clone, Debug)]
struct MacroEdit {
    index: u8,
    text: String,
    /// In characters. The text is ASCII, so also in bytes.
    cursor: usize,
    /// The text as it was read, so switching macros can tell an edit is
    /// about to be lost.
    saved: String,
    error: Option<String>,
}

impl MacroEdit {
    fn new(index: u8, text: String) -> Self {
        Self {
            index,
            cursor: text.len(),
            saved: text.clone(),
            text,
            error: None,
        }
    }

    fn dirty(&self) -> bool {
        self.text != self.saved
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    ResetKeymap,
    ResetEeprom,
    Bootloader,
}

pub struct Omakeeb {
    pub(crate) focus: FocusHandle,
    offered: Option<String>,
    devices: Vec<Discovered>,
    device_index: usize,
    pending: Option<Discovered>,
    session: Option<Session>,
    phase: Phase,
    layer: u8,
    selected: usize,
    group: Group,
    picking: bool,
    query: String,
    picker_index: usize,
    board_scroll: gpui_kit::ScrollHandle,
    picker_scroll: gpui_kit::ScrollHandle,
    /// Set when the highlight moves by key, like `reveal_selected`.
    reveal_pick: bool,
    /// Set by an arrow key, so the next frame scrolls the selected key into
    /// view. Only then: a scroll by hand is not undone on every redraw.
    reveal_selected: bool,
    overlay: Overlay,
    macro_edit: Option<MacroEdit>,
    status: String,
    status_error: bool,
    blocked: Option<String>,
    zoom: usize,
    plug_ticks: u8,
    access_requested: bool,
    watch: Option<gpui_kit::Task<()>>,
}

impl Omakeeb {
    pub fn new(launch: Launch, cx: &mut Context<'_, Self>) -> Self {
        let mut app = Self {
            focus: cx.focus_handle(),
            offered: None,
            devices: Vec::new(),
            device_index: 0,
            pending: None,
            session: None,
            phase: Phase::Devices,
            layer: 0,
            selected: 0,
            group: Group::Basic,
            picking: false,
            query: String::new(),
            picker_index: 0,
            board_scroll: gpui_kit::ScrollHandle::new(),
            picker_scroll: gpui_kit::ScrollHandle::new(),
            reveal_pick: false,
            reveal_selected: false,
            overlay: Overlay::None,
            macro_edit: None,
            status: String::new(),
            status_error: false,
            blocked: None,
            zoom: ui::ZOOM_HOME,
            plug_ticks: 0,
            access_requested: false,
            watch: None,
        };
        if launch.demo {
            match demo_session() {
                Ok(session) => {
                    app.session = Some(session);
                    app.phase = Phase::Board;
                    app.note("Sample keyboard. Nothing is plugged in.");
                }
                Err(err) => app.fail(err.to_string()),
            }
            return app;
        }
        if let Some(path) = launch.definition {
            match std::fs::read_to_string(&path) {
                Ok(text) => app.offered = Some(text),
                Err(err) => app.fail(format!("cannot read {}: {err}", path.display())),
            }
        }
        app.scan();
        app.arm_watch(cx);
        app
    }

    /// Keep looking for VIA interfaces for as long as the window is open.
    /// Plugging a keyboard in, or unplugging it, updates the window without
    /// starting omakeeb again.
    fn arm_watch(&mut self, cx: &mut Context<'_, Self>) {
        self.watch = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                let found = cx.background_executor().spawn(async { discover() }).await;
                let Ok(()) = this.update(cx, |this, cx| {
                    if this.on_plug(found) {
                        cx.notify();
                    }
                }) else {
                    break;
                };
            }
        }));
    }

    fn on_plug(&mut self, found: std::result::Result<Vec<Discovered>, Error>) -> bool {
        let Ok(found) = found else {
            return false;
        };
        if self.session.as_ref().is_some_and(|session| session.demo) {
            return false;
        }
        if let Some(path) = self
            .session
            .as_ref()
            .map(|session| session.identity.path.clone())
        {
            if found.iter().any(|device| device.path == path) {
                let changed = !same_devices(&self.devices, &found);
                self.devices = found;
                return changed;
            }
            self.session = None;
            self.pending = None;
            self.blocked = None;
            self.picking = false;
            self.devices = found;
            self.phase = Phase::Devices;
            self.note("The keyboard was unplugged.");
            if self.devices.len() == 1 {
                self.try_connect(0);
            }
            return true;
        }
        if same_devices(&self.devices, &found) {
            // Access can arrive after the node does, once udev tags it.
            if self.blocked.is_some() && found.len() == 1 {
                self.plug_ticks = self.plug_ticks.wrapping_add(1);
                if self.plug_ticks.is_multiple_of(4) {
                    self.try_connect(0);
                    return true;
                }
            }
            return false;
        }
        self.devices = found;
        self.blocked = None;
        self.plug_ticks = 0;
        if self.devices.len() == 1 {
            self.try_connect(0);
        } else if self.devices.is_empty() {
            self.pending = None;
            self.phase = Phase::Devices;
            self.note("No VIA keyboard found. Plug one in; it shows up on its own.");
        } else {
            self.pending = None;
            self.phase = Phase::Devices;
            self.device_index = 0;
            self.note(format!(
                "{} VIA keyboards. Enter opens the selected one.",
                self.devices.len()
            ));
        }
        true
    }

    fn scan(&mut self) {
        self.blocked = None;
        self.session = None;
        self.pending = None;
        match discover() {
            Ok(devices) => {
                self.devices = devices;
                if self.devices.len() == 1 {
                    self.try_connect(0);
                } else {
                    self.phase = Phase::Devices;
                    self.device_index = 0;
                    self.note(if self.devices.is_empty() {
                        "No VIA keyboard found. Plug one in; it shows up on its own.".to_owned()
                    } else {
                        format!(
                            "{} VIA keyboards. Enter opens the selected one.",
                            self.devices.len()
                        )
                    });
                }
            }
            Err(err) => {
                self.devices.clear();
                self.phase = Phase::Devices;
                self.fail(err.to_string());
            }
        }
    }

    fn try_connect(&mut self, index: usize) {
        let Some(device) = self.devices.get(index).cloned() else {
            return;
        };
        self.device_index = index;
        self.blocked = None;
        if let Some(json) = self.offered.clone() {
            match remember(&json, device.vendor_id, device.product_id) {
                Ok((definition, path)) => {
                    self.offered = None;
                    self.note(format!(
                        "Saved this layout for {:04X}:{:04X} at {}",
                        device.vendor_id,
                        device.product_id,
                        path.display()
                    ));
                    self.open_with(device, definition);
                }
                Err(err) => {
                    self.pending = Some(device);
                    self.phase = Phase::NeedLayout;
                    self.fail(err.to_string());
                }
            }
            return;
        }
        let found = Catalog::load(&definitions_dir())
            .find(device.vendor_id, device.product_id)
            .cloned();
        if let Some(definition) = found {
            self.open_with(device, definition);
        } else {
            self.pending = Some(device);
            self.phase = Phase::NeedLayout;
            self.note(
                "This keyboard does not send its layout. Choose a VIA definition, and it will be kept for next time."
                    .to_owned(),
            );
        }
    }

    fn open_with(&mut self, device: Discovered, definition: Definition) {
        let link = match self.open_link(&device) {
            Ok(link) => link,
            Err(Error::Permission { .. }) => {
                self.phase = Phase::Devices;
                self.blocked = Some(
                    "The keyboard is connected, and this user still cannot open it. Install omakeeb again; that step grants access."
                        .to_owned(),
                );
                self.fail(format!(
                    "{} is plugged in, and its configuration interface could not be opened.",
                    device.label()
                ));
                return;
            }
            Err(err) => {
                self.phase = Phase::Devices;
                self.fail(err.to_string());
                return;
            }
        };
        let identity = Identity {
            path: device.path.clone(),
            vendor_id: device.vendor_id,
            product_id: device.product_id,
            manufacturer: device.manufacturer.clone(),
            product: device.product.clone(),
        };
        match Session::connect(Box::new(link), identity, definition, false) {
            Ok(session) => {
                self.pending = None;
                self.layer = 0;
                self.selected = 0;
                self.picking = false;
                self.query.clear();
                self.session = Some(session);
                self.phase = Phase::Board;
                if !self.status_error {
                    self.note("Changes write to the keyboard immediately.".to_owned());
                }
            }
            Err(err) => {
                self.phase = Phase::Devices;
                self.fail(err.to_string());
            }
        }
    }

    fn open_link(&mut self, device: &Discovered) -> std::result::Result<HidLink, Error> {
        match HidLink::open(&device.path) {
            Err(Error::Permission { .. }) if !self.access_requested => {
                self.access_requested = true;
                self.note(format!("Allowing access to {}…", device.label()));
                if grant_hid_access() {
                    return HidLink::open(&device.path);
                }
                Err(Error::Permission {
                    path: device.path.clone(),
                    detail: "access was not granted".to_owned(),
                })
            }
            other => other,
        }
    }

    fn choose_layout(&mut self) {
        let Some(device) = self.pending.clone() else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_title("VIA keyboard definition")
            .add_filter("VIA definition", &["json"])
            .pick_file()
        else {
            return;
        };
        self.accept_layout_file(&device, &path);
    }

    fn accept_layout_file(&mut self, device: &Discovered, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                self.fail(format!("cannot read {}: {err}", path.display()));
                return;
            }
        };
        match remember(&text, device.vendor_id, device.product_id) {
            Ok((definition, saved)) => {
                self.note(format!(
                    "Saved {} for {:04X}:{:04X}. Next time this keyboard opens directly.",
                    saved.display(),
                    device.vendor_id,
                    device.product_id
                ));
                self.open_with(device.clone(), definition);
            }
            Err(err) => self.fail(format!("{}: {err}", path.display())),
        }
    }

    pub(crate) fn select_key(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.selected = index;
        self.picking = false;
        self.refocus(window, cx);
        cx.notify();
    }

    fn refocus(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        window.focus(&self.focus, cx);
    }

    fn note(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_error = false;
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_error = true;
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if event.keystroke.modifiers.platform {
            return;
        }
        let key = event.keystroke.key.as_ref();
        let shift = event.keystroke.modifiers.shift;
        let control = event.keystroke.modifiers.control;
        if control && !shift && !event.keystroke.modifiers.alt {
            match key {
                "=" | "+" => self.zoom(1, window),
                "-" => self.zoom(-1, window),
                "0" => {
                    self.zoom = ui::ZOOM_HOME;
                    self.apply_zoom(window);
                }
                "s" => self.save_keymap(),
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if control || event.keystroke.modifiers.alt {
            return;
        }
        if event.is_held && matches!(key, "enter" | "q" | "escape" | "r" | "o") {
            return;
        }
        if key == "q" && !self.picking && self.overlay == Overlay::None {
            cx.quit();
            cx.stop_propagation();
            return;
        }
        let handled = match self.overlay {
            Overlay::Help => self.key_help(key),
            Overlay::Confirm(action) => self.key_confirm(key, action),
            Overlay::Macro => self.key_macro(key, shift, event.keystroke.key_char.as_deref()),
            Overlay::None if self.picking => self.key_picker(key, shift),
            Overlay::None => self.key_board(key, shift),
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn key_help(&mut self, key: &str) -> bool {
        if key == "escape" || key == "?" || key == "/" {
            self.overlay = Overlay::None;
        }
        true
    }

    fn key_confirm(&mut self, key: &str, action: Confirm) -> bool {
        match key {
            "escape" => self.overlay = Overlay::None,
            "enter" => {
                self.overlay = Overlay::None;
                self.commit(action);
            }
            _ => {}
        }
        true
    }

    /// A one-line editor: the typed character goes in at the cursor, so it
    /// follows the user's own keyboard layout rather than a US map.
    fn key_macro(&mut self, key: &str, shift: bool, typed: Option<&str>) -> bool {
        match key {
            "escape" => self.close_macro(),
            "enter" => self.save_macro(),
            "tab" => self.step_macro(if shift { -1 } else { 1 }),
            _ => {
                let Some(edit) = self.macro_edit.as_mut() else {
                    self.overlay = Overlay::None;
                    return true;
                };
                match key {
                    "left" => edit.cursor = edit.cursor.saturating_sub(1),
                    "right" => edit.cursor = (edit.cursor + 1).min(edit.text.len()),
                    "home" => edit.cursor = 0,
                    "end" => edit.cursor = edit.text.len(),
                    "backspace" if edit.cursor > 0 => {
                        edit.cursor -= 1;
                        edit.text.remove(edit.cursor);
                        edit.error = None;
                    }
                    "delete" if edit.cursor < edit.text.len() => {
                        edit.text.remove(edit.cursor);
                        edit.error = None;
                    }
                    _ => {
                        // Printable ASCII only: that is what a macro can type,
                        // and it keeps the cursor a byte index.
                        let ch = typed
                            .filter(|typed| typed.chars().count() == 1)
                            .and_then(|typed| typed.chars().next())
                            .or_else(|| typed_char(key, shift));
                        if let Some(ch) = ch.filter(|ch| (' '..='~').contains(ch)) {
                            edit.text.insert(edit.cursor, ch);
                            edit.cursor += 1;
                            edit.error = None;
                        }
                    }
                }
            }
        }
        true
    }

    /// The selected key's macro when it is one, otherwise the first.
    fn open_macro(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let Some(macros) = &session.macros else {
            self.fail("This keyboard's firmware has no dynamic macros.");
            return;
        };
        let count = macros.count();
        let index = self
            .selected_code()
            .and_then(macro_index)
            .filter(|index| *index < count)
            .unwrap_or(0);
        self.edit_macro(index);
    }

    fn edit_macro(&mut self, index: u8) {
        let Some(actions) = self
            .session
            .as_ref()
            .and_then(|session| session.macros.as_ref())
            .and_then(|macros| macros.macros.get(usize::from(index)))
        else {
            return;
        };
        self.macro_edit = Some(MacroEdit::new(index, macros::to_text(actions)));
        self.overlay = Overlay::Macro;
    }

    fn step_macro(&mut self, delta: isize) {
        let Some(edit) = self.macro_edit.as_mut() else {
            return;
        };
        if edit.dirty() {
            edit.error =
                Some("Save with enter, or close with escape to drop the change.".to_owned());
            return;
        }
        let count = self
            .session
            .as_ref()
            .and_then(|session| session.macros.as_ref())
            .map_or(1, |macros| macros.count().max(1)) as isize;
        let next = (edit.index as isize + delta).rem_euclid(count) as u8;
        self.edit_macro(next);
    }

    fn close_macro(&mut self) {
        if self.macro_edit.as_ref().is_some_and(MacroEdit::dirty) {
            self.note("Macro left as it was on the keyboard.");
        }
        self.macro_edit = None;
        self.overlay = Overlay::None;
    }

    fn save_macro(&mut self) {
        let Some(edit) = self.macro_edit.as_mut() else {
            self.overlay = Overlay::None;
            return;
        };
        let actions = match macros::from_text(&edit.text) {
            Ok(actions) => actions,
            Err(err) => {
                edit.error = Some(err.to_string());
                return;
            }
        };
        let index = edit.index;
        // Unplugged while the editor was open.
        let Some(session) = self.session.as_mut() else {
            self.close_macro();
            return;
        };
        match session.set_macro(index, actions) {
            Ok(()) => {
                let (used, size) = session
                    .macros
                    .as_ref()
                    .map_or((0, 0), |macros| (macros.used(), macros.size));
                self.macro_edit = None;
                self.overlay = Overlay::None;
                self.note(format!(
                    "M{index} saved on the keyboard · {used} of {size} bytes used"
                ));
            }
            Err(err) => {
                if let Some(edit) = self.macro_edit.as_mut() {
                    edit.error = Some(err.to_string());
                }
            }
        }
    }

    fn key_picker(&mut self, key: &str, shift: bool) -> bool {
        match key {
            "escape" => {
                self.picking = false;
                self.query.clear();
            }
            "enter" => self.assign_highlighted(),
            "backspace" => {
                self.query.pop();
                self.picker_index = 0;
                self.reveal_pick = true;
            }
            "up" => self.move_picker(-1),
            "down" => self.move_picker(1),
            "left" | "right" if self.query.is_empty() => {
                self.step_group(if key == "right" { 1 } else { -1 });
            }
            "tab" => self.step_group(if shift { -1 } else { 1 }),
            _ => {
                if let Some(ch) = typed_char(key, shift) {
                    self.query.push(ch);
                    self.picker_index = 0;
                    self.reveal_pick = true;
                }
            }
        }
        true
    }

    fn key_board(&mut self, key: &str, shift: bool) -> bool {
        if self.phase == Phase::NeedLayout {
            return match key {
                "enter" | "o" => {
                    self.choose_layout();
                    true
                }
                "r" => {
                    self.scan();
                    true
                }
                "?" | "/" if shift || key == "?" => {
                    self.overlay = Overlay::Help;
                    true
                }
                _ => false,
            };
        }
        match key {
            "?" => self.overlay = Overlay::Help,
            "/" if shift => self.overlay = Overlay::Help,
            "escape" => {
                if self.phase == Phase::Board && self.session.is_some() {
                    self.picking = false;
                }
            }
            "r" => self.scan(),
            "enter" if self.phase == Phase::Devices && !self.devices.is_empty() => {
                self.try_connect(self.device_index);
            }
            "enter" if self.phase == Phase::Board => self.open_picker(),
            "m" if self.phase == Phase::Board => self.open_macro(),
            "up" if self.phase == Phase::Devices => self.step_device(-1),
            "down" if self.phase == Phase::Devices => self.step_device(1),
            "up" => self.move_selection((0.0, -1.0)),
            "down" => self.move_selection((0.0, 1.0)),
            "left" => self.move_selection((-1.0, 0.0)),
            "right" => self.move_selection((1.0, 0.0)),
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" => {
                let layer = key.as_bytes()[0] - b'1';
                self.set_layer(layer);
            }
            "-" | "=" if self.phase == Phase::Board => {
                if shift {
                    self.adjust_light(if key == "=" { 8 } else { -8 }, LightField::Speed);
                } else {
                    self.adjust_light(if key == "=" { 8 } else { -8 }, LightField::Brightness);
                }
            }
            "[" | "]" if self.phase == Phase::Board => {
                self.adjust_light(if key == "]" { 1 } else { -1 }, LightField::Effect);
            }
            ";" | "'" if self.phase == Phase::Board => {
                self.adjust_light(if key == "'" { 8 } else { -8 }, LightField::Hue);
            }
            _ => return false,
        }
        true
    }

    fn zoom(&mut self, delta: isize, window: &mut Window) {
        let next = self.zoom as isize + delta;
        self.zoom = next.clamp(0, ui::ZOOM.len() as isize - 1) as usize;
        self.apply_zoom(window);
    }

    fn apply_zoom(&self, window: &mut Window) {
        window.set_rem_size(px(ui::BASE_REM * ui::ZOOM[self.zoom]));
    }

    fn step_device(&mut self, delta: isize) {
        if self.devices.is_empty() {
            return;
        }
        let len = self.devices.len() as isize;
        self.device_index = (self.device_index as isize + delta).rem_euclid(len) as usize;
    }

    fn step_group(&mut self, delta: isize) {
        let index = Group::ALL
            .iter()
            .position(|group| *group == self.group)
            .unwrap_or(0) as isize;
        let len = Group::ALL.len() as isize;
        self.group = Group::ALL[(index + delta).rem_euclid(len) as usize];
        self.picker_index = 0;
        self.reveal_pick = true;
    }

    fn move_picker(&mut self, delta: isize) {
        let len = self.filtered().len() as isize;
        if len == 0 {
            return;
        }
        self.picker_index = (self.picker_index as isize + delta).rem_euclid(len) as usize;
        self.reveal_pick = true;
    }

    fn move_selection(&mut self, direction: (f32, f32)) {
        let Some(drawn) = self.drawn() else {
            return;
        };
        if let Some(next) = board::neighbor(&drawn, self.selected, direction) {
            self.selected = next;
            self.reveal_selected = true;
        }
    }

    fn set_layer(&mut self, layer: u8) {
        let Some(session) = &self.session else {
            return;
        };
        if layer >= session.keymap.layers {
            return;
        }
        self.layer = layer;
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let count = self
            .session
            .as_ref()
            .map(|session| {
                session
                    .definition
                    .visible_keys(session.layout_options)
                    .iter()
                    .filter(|key| key.selectable())
                    .count()
            })
            .unwrap_or(0);
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    fn open_picker(&mut self) {
        self.picking = true;
        self.query.clear();
        self.picker_index = 0;
        if let Some(code) = self.selected_code() {
            if let Some(index) = self.filtered().iter().position(|item| item.code == code) {
                self.picker_index = index;
            }
        }
        self.reveal_pick = true;
    }

    fn assign_highlighted(&mut self) {
        let Some(item) = self.filtered().get(self.picker_index).cloned() else {
            if let Some(code) = parse_keycode(&self.query) {
                self.assign(code);
            } else if !self.query.is_empty() {
                self.fail(format!("No keycode matches {}", self.query));
            }
            return;
        };
        self.assign(item.code);
    }

    fn assign(&mut self, code: u16) {
        let Some((row, col)) = self.selected_position() else {
            return;
        };
        let layer = self.layer;
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let custom = session.definition.custom.clone();
        if let Err(err) = session.set_keycode(layer, row, col, code) {
            self.fail(err.to_string());
            return;
        }
        self.picking = false;
        self.query.clear();
        self.note(format!(
            "Layer {} · row {row} column {col} is {}",
            layer,
            long_name(code, &custom)
        ));
    }

    fn selected_position(&self) -> Option<(u8, u8)> {
        let session = self.session.as_ref()?;
        session
            .definition
            .visible_keys(session.layout_options)
            .into_iter()
            .filter(|key| key.selectable())
            .nth(self.selected)
            .and_then(|key| key.row.zip(key.col))
    }

    fn selected_code(&self) -> Option<u16> {
        let (row, col) = self.selected_position()?;
        self.session.as_ref()?.keymap.get(self.layer, row, col)
    }

    fn drawn(&self) -> Option<Vec<board::Cap>> {
        let session = self.session.as_ref()?;
        Some(board::caps(
            &session.definition,
            &session.keymap,
            self.layer,
            session.layout_options,
        ))
    }

    fn filtered(&self) -> Vec<Item> {
        let (custom, layers) = self
            .session
            .as_ref()
            .map(|session| (session.definition.custom.clone(), session.keymap.layers))
            .unwrap_or_else(|| (Vec::new(), 4));
        let stored = self
            .session
            .as_ref()
            .and_then(|session| session.macros.as_ref());
        let macro_count = stored.map_or(0, omakeeb_core::Macros::count);
        let mut items = if self.query.is_empty() {
            items_for(self.group, &custom, layers, macro_count)
        } else {
            Group::ALL
                .into_iter()
                .flat_map(|group| items_for(group, &custom, layers, macro_count))
                .collect()
        };
        // A macro's name alone does not say what it types.
        if let Some(stored) = stored {
            for item in &mut items {
                if let Some(actions) = macro_index(item.code)
                    .and_then(|index| stored.macros.get(usize::from(index)))
                    .filter(|actions| !actions.is_empty())
                {
                    item.name = macro_preview(&macros::to_text(actions));
                }
            }
        }
        if !self.query.is_empty() {
            let needle = self.query.to_ascii_lowercase();
            items.retain(|item| {
                item.name.to_ascii_lowercase().contains(&needle)
                    || item.short.to_ascii_lowercase().contains(&needle)
            });
            if let Some(code) = parse_keycode(&self.query)
                && !items.iter().any(|item| item.code == code)
            {
                items.insert(
                    0,
                    Item {
                        code,
                        name: long_name(code, &custom),
                        short: short_name(code, &custom),
                        group: Group::Quantum,
                    },
                );
            }
        }
        items
    }

    fn adjust_light(&mut self, delta: i16, field: LightField) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.lighting.is_empty() {
            return;
        }
        let result = match field {
            LightField::Brightness => session.adjust_brightness(0, delta),
            LightField::Effect => session.adjust_effect(0, delta),
            LightField::Speed => session.adjust_speed(0, delta),
            LightField::Hue => session.adjust_hue(0, delta),
        };
        match result {
            Ok(()) => {
                let light = session.lighting[0];
                self.note(format!(
                    "{} · brightness {} · {}",
                    light.channel.label(),
                    light.brightness,
                    effect_label(light.channel, light.effect)
                ));
            }
            Err(err) => self.fail(err.to_string()),
        }
    }

    fn set_layout_choice(&mut self, group: usize, choice: u32) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let value = match session
            .definition
            .with_choice(session.layout_options, group, choice)
        {
            Ok(value) => value,
            Err(err) => {
                self.fail(err.to_string());
                return;
            }
        };
        if let Err(err) = session.set_layout_options(value) {
            self.fail(err.to_string());
            return;
        }
        self.clamp_selection();
        self.note("Layout option saved on the keyboard.");
    }

    fn save_keymap(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let dir = omakeeb_core::keymaps_dir();
        if let Err(err) = omakeeb_core::ensure_dir(&dir) {
            self.fail(err.to_string());
            return;
        }
        let slug = slug(&session.definition.name);
        let path = dir.join(format!("{slug}.json"));
        let text = session.keymap.to_json(
            &session.definition.name,
            session.identity.vendor_id,
            session.identity.product_id,
        );
        match std::fs::write(&path, text) {
            Ok(()) => self.note(format!("Saved keymap to {}", path.display())),
            Err(err) => self.fail(format!("cannot write {}: {err}", path.display())),
        }
    }

    fn commit(&mut self, action: Confirm) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let result = match action {
            Confirm::ResetKeymap => session
                .reset_keymap()
                .map(|()| "Keymap reset to the firmware default.".to_owned()),
            Confirm::ResetEeprom => session.reset_eeprom().map(|()| {
                "EEPROM cleared. The keyboard may restart; press r if it disappears.".to_owned()
            }),
            Confirm::Bootloader => session
                .jump_bootloader()
                .map(|()| "The keyboard is in its bootloader and has disconnected.".to_owned()),
        };
        match result {
            Ok(text) => {
                let disconnect = matches!(action, Confirm::ResetEeprom | Confirm::Bootloader);
                self.note(text);
                if disconnect && self.session.as_ref().is_some_and(|session| !session.demo) {
                    self.session = None;
                    self.phase = Phase::Devices;
                }
            }
            Err(err) => self.fail(err.to_string()),
        }
    }
}

#[derive(Clone, Copy)]
enum LightField {
    Brightness,
    Effect,
    Speed,
    Hue,
}

impl Render for Omakeeb {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.apply_zoom(window);
        let stacked = stacked(window);
        let theme = cx.omarchy().clone();
        let status_color = if self.status_error {
            theme.danger
        } else {
            theme.secondary
        };
        div()
            .id("omakeeb-root")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(text::BODY)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.refocus(window, cx)),
            )
            .child(self.top_bar(stacked, cx))
            .child(self.middle(stacked, window, cx))
            .child(
                div()
                    .h(size::STATUS)
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .px(space::LG)
                    .border_t_1()
                    .border_color(theme.divider())
                    .text_size(text::CAPTION)
                    .text_color(status_color)
                    .child(div().min_w_0().truncate().child(self.status.clone())),
            )
            .children(self.overlay_view(cx))
    }
}

impl Omakeeb {
    /// Too narrow for everything, the title gives way first, then the layer
    /// buttons scroll. Nothing here wraps: the bar keeps one fixed height.
    fn top_bar(&self, stacked: bool, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.omarchy();
        let title = self
            .session
            .as_ref()
            .map(|session| session.definition.name.clone())
            .or_else(|| self.pending.as_ref().map(Discovered::label))
            .unwrap_or_else(|| "No keyboard".to_owned());
        let mut bar = div()
            .flex()
            .items_center()
            .gap(space::MD)
            .px(space::LG)
            .h(size::BAR)
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.divider())
            .child(
                svg()
                    .path(crate::MARK)
                    .flex_shrink_0()
                    .size(size::MARK)
                    .text_color(theme.foreground),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(text::HEADING)
                    .font_weight(FontWeight::BOLD)
                    .child("omakeeb"),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.secondary)
                    .child(title),
            );
        if let Some(session) = &self.session {
            if !stacked {
                bar = bar.child(div().flex_shrink_0().child(badge(
                    format!("protocol {}", session.protocol),
                    Status::Neutral,
                    cx,
                )));
            }
            if session.demo {
                bar = bar.child(
                    div()
                        .flex_shrink_0()
                        .child(badge("demo", Status::Warning, cx)),
                );
            }
            let mut layers = div()
                .id("layers")
                .flex()
                .items_center()
                .gap(space::MD)
                .min_w_0()
                .overflow_x_scroll();
            for layer in 0..session.keymap.layers {
                let selected = layer == self.layer && self.phase == Phase::Board;
                let label = format!("{}", layer + 1);
                layers = layers.child(
                    div().flex_shrink_0().child(
                        button(
                            SharedString::from(format!("layer-{layer}")),
                            label,
                            if selected {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            },
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.set_layer(layer);
                                this.refocus(window, cx);
                                cx.notify();
                            },
                        )),
                    ),
                );
            }
            bar = bar.child(layers);
        }
        bar.child(div().flex_1()).child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_size(text::CAPTION)
                .text_color(theme.secondary)
                .child("? help"),
        )
    }

    /// Side by side when there is room. Otherwise the panel goes under the
    /// board and the two scroll together, because a tiled window can be any
    /// size: Hyprland does not honour the minimum size a window asks for.
    fn middle(
        &mut self,
        stacked: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> gpui_kit::AnyElement {
        let main = self.main_column(stacked, window, cx).into_any_element();
        let list = picker_height(stacked, window);
        let panel = self.side_panel(stacked, list, cx).into_any_element();
        if stacked {
            div()
                .id("stack")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(main)
                .child(panel)
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(main)
                .child(panel)
                .into_any_element()
        }
    }

    fn main_column(
        &mut self,
        stacked: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let mut column = div().min_w_0().flex().flex_col().p(space::LG);
        // Stacked, the column is as tall as what it holds and the stack scrolls.
        column = if stacked {
            column.flex_none()
        } else {
            column.flex_1()
        };
        if let Some(blocked) = &self.blocked {
            column = column.child(alert(blocked.clone(), Status::Error, cx));
        }
        column.child(match self.phase {
            Phase::NeedLayout => self.need_layout(cx).into_any_element(),
            Phase::Devices => self.device_list(cx).into_any_element(),
            Phase::Board => self
                .board_area(stacked, window, &theme, cx)
                .into_any_element(),
        })
    }

    fn need_layout(&mut self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let device = self.pending.clone();
        let (name, ids) = device
            .as_ref()
            .map(|device| {
                (
                    device.label(),
                    format!("{:04X}:{:04X}", device.vendor_id, device.product_id),
                )
            })
            .unwrap_or_else(|| ("Keyboard".to_owned(), String::new()));
        let directory = definitions_dir();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(space::MD)
            .child(empty_state(
                format!("{name} has no saved layout"),
                "VIA keyboards do not send their layout. Choose the manufacturer's JSON definition. omakeeb saves it under this keyboard's USB ids, and the next plug-in opens directly.",
                cx,
            ))
            .child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(cx.omarchy().secondary)
                    .child(ids),
            )
            .child(
                button("choose-layout", "Choose definition…", ButtonVariant::Primary, cx).on_click(
                    cx.listener(|this, _, window, cx| {
                        this.choose_layout();
                        this.refocus(window, cx);
                        cx.notify();
                    }),
                ),
            )
            .child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(cx.omarchy().secondary)
                    .child(format!(
                        "Or copy a JSON file whose vendor and product ids match into {}",
                        directory.display()
                    )),
            )
            .child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(cx.omarchy().secondary)
                    .child("Enter chooses a file. r looks again."),
            )
    }

    fn device_list(&mut self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.devices.is_empty() {
            return empty_state(
                "No VIA keyboard",
                "Plug in a keyboard running QMK with VIA enabled. It shows up on its own. omakeeb --demo opens a sample board.",
                cx,
            )
            .into_any_element();
        }
        let theme = cx.omarchy().clone();
        let selected = self.device_index;
        div()
            .flex()
            .flex_col()
            .gap(space::SM)
            .children(
                self.devices
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(index, device)| {
                        let on = index == selected;
                        div()
                            .id(SharedString::from(format!("device-{index}")))
                            .flex()
                            .flex_col()
                            .gap(space::XS)
                            .p(space::MD)
                            .border_1()
                            .border_color(if on { theme.warning } else { theme.border })
                            .bg(if on { theme.surface } else { theme.background })
                            .child(device.label())
                            .child(
                                div()
                                    .text_size(text::CAPTION)
                                    .text_color(theme.secondary)
                                    .child(format!(
                                        "{:04X}:{:04X}",
                                        device.vendor_id, device.product_id
                                    )),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.try_connect(index);
                                    this.refocus(window, cx);
                                    cx.notify();
                                }),
                            )
                    }),
            )
            .into_any_element()
    }

    fn board_area(
        &mut self,
        stacked: bool,
        window: &mut Window,
        theme: &gpui_omarchy::Theme,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let reveal = std::mem::take(&mut self.reveal_selected);
        let Some(session) = &self.session else {
            return empty_state("No keymap", "The keyboard disconnected.", cx).into_any_element();
        };
        let drawn = board::caps(
            &session.definition,
            &session.keymap,
            self.layer,
            session.layout_options,
        );
        let (units_w, units_h) = session.definition.bounds();
        let (avail_w, avail_h) = board_space(stacked, window);
        let rem = board::pixels(window.rem_size());
        let unit = (avail_w / units_w)
            .min(avail_h / units_h)
            .clamp(size::KEY_MIN.0 * rem, size::KEY_MAX.0 * rem);
        let width = units_w * unit;
        let height = units_h * unit;
        let selected = self.selected;
        if reveal && let Some(cap) = drawn.iter().find(|cap| cap.selectable == Some(selected)) {
            // Only an axis that overflows has been start-aligned, so only there
            // do key units map straight onto the scroll offset.
            let view = self.board_scroll.bounds().size;
            let offset = self.board_scroll.offset();
            let (view_w, view_h) = (board::pixels(view.width), board::pixels(view.height));
            let (mut x, mut y) = (-board::pixels(offset.x), -board::pixels(offset.y));
            if width > view_w {
                x = board::reveal(x, view_w, cap.x * unit, cap.w * unit);
            }
            if height > view_h {
                y = board::reveal(y, view_h, cap.y * unit, cap.h * unit);
            }
            self.board_scroll
                .set_offset(gpui_kit::point(px(-x), px(-y)));
        }
        let mut area = div().id("board-area").flex().min_w_0().bg(theme.background);
        // Stacked, the board is as tall as its keys and only scrolls across,
        // so a vertical wheel over it still scrolls the stack.
        area = if stacked {
            area.w_full().h(px(height)).flex_none().overflow_x_scroll()
        } else {
            area.flex_1().min_h_0().overflow_scroll()
        };
        // Centring something larger than its scroll container pushes its
        // start out of reach, so an axis that overflows aligns to the start.
        // Half a pixel absorbs rounding in the layout.
        area = if width <= avail_w + 0.5 {
            area.justify_center()
        } else {
            area.justify_start()
        };
        area = if stacked || height <= avail_h + 0.5 {
            area.items_center()
        } else {
            area.items_start()
        };
        area.track_scroll(&self.board_scroll)
            .child(board::keyboard(
                &drawn, unit, width, height, selected, window, cx,
            ))
            .into_any_element()
    }

    fn side_panel(
        &mut self,
        stacked: bool,
        list: Option<gpui_kit::Pixels>,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let mut panel = div()
            .id("side-panel")
            .flex()
            .flex_col()
            .gap(space::MD)
            .p(space::LG)
            .border_color(theme.divider());
        // Beside the board the panel keeps its width and scrolls on its own.
        // Under it, the panel is full width and the stack does the scrolling.
        panel = if stacked {
            panel.w_full().flex_none().border_t_1()
        } else {
            panel
                .w(size::PANEL)
                .flex_shrink_0()
                .border_l_1()
                .overflow_y_scroll()
        };
        if self.phase != Phase::Board {
            return panel
                .child(
                    div()
                        .text_size(text::TITLE)
                        .font_weight(FontWeight::BOLD)
                        .child(if self.phase == Phase::NeedLayout {
                            "Layout"
                        } else {
                            "Keyboards"
                        }),
                )
                .child(separator(cx));
        }
        let code = self.selected_code();
        let custom = self
            .session
            .as_ref()
            .map(|session| session.definition.custom.clone())
            .unwrap_or_default();
        let (row, col) = self.selected_position().unwrap_or((0, 0));
        let legend = code
            .map(|code| short_name(code, &custom))
            .unwrap_or_default();
        let full = code
            .map(|code| long_name(code, &custom))
            .unwrap_or_else(|| "—".to_owned());
        panel = panel
            .child(
                div()
                    .text_size(text::DISPLAY)
                    .font_family(theme.mono_font.clone())
                    .line_height(relative(1.0))
                    .child(if legend.is_empty() {
                        "·".to_owned()
                    } else {
                        legend
                    }),
            )
            .child(div().text_color(theme.secondary).child(full))
            .child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(theme.secondary)
                    .child(format!("Layer {} · row {row} · column {col}", self.layer)),
            );
        if let Some(text) = self.selected_macro_text() {
            panel = panel.child(
                div()
                    .font_family(theme.mono_font.clone())
                    .text_size(text::CAPTION)
                    .child(if text.is_empty() {
                        "Empty macro. Press m to write it.".to_owned()
                    } else {
                        text
                    }),
            );
        }
        panel = panel.child(separator(cx));
        panel = self.layout_controls(panel, cx);
        panel = self.picker(panel, theme.clone(), list, cx);
        panel = self.lighting_controls(panel, cx);
        panel = panel.child(separator(cx)).child(
            button("reset-keymap", "Reset keymap…", ButtonVariant::Danger, cx).on_click(
                cx.listener(|this, _, window, cx| {
                    this.overlay = Overlay::Confirm(Confirm::ResetKeymap);
                    this.refocus(window, cx);
                    cx.notify();
                }),
            ),
        );
        let demo = self.session.as_ref().is_some_and(|session| session.demo);
        if !demo {
            panel = panel
                .child(
                    button("reset-eeprom", "Clear EEPROM…", ButtonVariant::Danger, cx).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.overlay = Overlay::Confirm(Confirm::ResetEeprom);
                            this.refocus(window, cx);
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button("bootloader", "Bootloader…", ButtonVariant::Danger, cx).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.overlay = Overlay::Confirm(Confirm::Bootloader);
                            this.refocus(window, cx);
                            cx.notify();
                        }),
                    ),
                );
        }
        panel
    }

    fn layout_controls(
        &mut self,
        mut panel: gpui_kit::Stateful<gpui_kit::Div>,
        cx: &mut Context<'_, Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let Some(session) = &self.session else {
            return panel;
        };
        if session.definition.groups.is_empty() {
            return panel;
        }
        let groups = session.definition.groups.clone();
        let options = session.layout_options;
        let definition = session.definition.clone();
        panel = panel.child(
            div()
                .text_size(text::CAPTION)
                .text_color(cx.omarchy().secondary)
                .child("Layout"),
        );
        for (group_index, group) in groups.iter().enumerate() {
            let current = definition.choice(options, group_index);
            let mut row = div().flex().flex_wrap().gap(space::XS).items_center();
            row = row.child(
                div()
                    .text_size(text::CAPTION)
                    .w(rems_width_label())
                    .child(group.label.clone()),
            );
            for (choice_index, choice) in group.choices.iter().cloned().enumerate() {
                let on = current == choice_index as u32;
                row = row.child(
                    button(
                        SharedString::from(format!("layout-{group_index}-{choice_index}")),
                        choice,
                        if on {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        },
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_layout_choice(group_index, choice_index as u32);
                        this.refocus(window, cx);
                        cx.notify();
                    })),
                );
            }
            panel = panel.child(row);
        }
        panel.child(separator(cx))
    }

    fn picker(
        &mut self,
        mut panel: gpui_kit::Stateful<gpui_kit::Div>,
        theme: gpui_omarchy::Theme,
        list_height: Option<gpui_kit::Pixels>,
        cx: &mut Context<'_, Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let groups = Group::ALL;
        let current = self.group;
        let mut chips = div().flex().flex_wrap().gap(space::XS);
        for group in groups {
            let on = group == current && self.query.is_empty();
            chips = chips.child(
                button(
                    SharedString::from(format!("group-{}", group.label())),
                    group.label(),
                    if on {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    },
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.group = group;
                    this.query.clear();
                    this.picker_index = 0;
                    this.picking = true;
                    this.reveal_pick = true;
                    this.refocus(window, cx);
                    cx.notify();
                })),
            );
        }
        // Not a text input: keys already come to the window, and the picker
        // takes them while it is open. Clicking here opens it.
        let searching = self.picking;
        let search = div()
            .id("picker-search")
            .flex()
            .items_center()
            .gap(space::SM)
            .px(space::SM)
            .py(space::XS)
            .border_1()
            .border_color(if searching {
                theme.accent
            } else {
                theme.border
            })
            .child(
                icon(IconName::Search)
                    .size(text::BODY)
                    .text_color(theme.secondary),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font.clone())
                    .text_color(if self.query.is_empty() {
                        theme.secondary
                    } else {
                        theme.foreground
                    })
                    .child(match (self.query.is_empty(), searching) {
                        (true, true) => "Type a keycode, e.g. vol or mo(1)".to_owned(),
                        (true, false) => "Search keycodes".to_owned(),
                        (false, true) => format!("{}▏", self.query),
                        (false, false) => self.query.clone(),
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !this.picking {
                        this.open_picker();
                    }
                    this.refocus(window, cx);
                    cx.notify();
                }),
            );
        panel = panel.child(search).child(chips);
        let items = self.filtered();
        let stored = self
            .session
            .as_ref()
            .and_then(|session| session.macros.as_ref());
        if let Some(stored) = stored
            && current == Group::Macro
            && self.query.is_empty()
        {
            panel = panel.child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(theme.secondary)
                    .child(format!(
                        "{} of {} bytes used. The pencil edits a macro.",
                        stored.used(),
                        stored.size
                    )),
            );
        }
        let macro_count = stored.map_or(0, omakeeb_core::Macros::count);
        let highlight = self.picker_index;
        let shown = items.len().min(80);
        // The list scrolls on its own. Beside the board it takes whatever
        // height the panel has left, which puts the controls after it at the
        // bottom; under the board it is capped, see `picker_height`.
        let mut list = div()
            .id("picker-list")
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.picker_scroll);
        list = match list_height {
            Some(height) => list.max_h(height),
            None => list.flex_1().min_h(size::PICKER_LIST_MIN),
        };
        if std::mem::take(&mut self.reveal_pick) {
            self.picker_scroll
                .scroll_to_item(highlight.min(shown.saturating_sub(1)));
        }
        for (index, item) in items.iter().take(shown).cloned().enumerate() {
            let on = self.picking && index == highlight;
            let editable = macro_index(item.code).filter(|slot| *slot < macro_count);
            // A macro's preview can be long: it gives way, the pencil does not.
            let mut trailing = div()
                .flex()
                .items_center()
                .justify_end()
                .gap(space::SM)
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(text::CAPTION)
                        .text_color(theme.secondary)
                        .child(item.name.clone()),
                );
            if let Some(slot) = editable {
                trailing = trailing.child(
                    with_tooltip(
                        icon_button(
                            SharedString::from(format!("edit-macro-{slot}")),
                            IconName::Pencil,
                            format!("Edit M{slot}"),
                            ButtonVariant::Secondary,
                            cx,
                        ),
                        format!("Edit M{slot}"),
                    )
                    // The row assigns on mouse down; the pencil must not.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_macro(slot);
                        this.refocus(window, cx);
                        cx.notify();
                    })),
                );
            }
            list = list.child(
                div()
                    .id(SharedString::from(format!("pick-{index}-{}", item.code)))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(space::SM)
                    .py(space::XS)
                    .bg(if on {
                        theme.warning.opacity(0.18)
                    } else {
                        theme.background
                    })
                    .border_l_1()
                    .border_color(if on { theme.warning } else { theme.background })
                    .gap(space::SM)
                    .child(div().flex_shrink_0().child(item.short.clone()))
                    .child(trailing)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.picker_index = index;
                            this.assign(item.code);
                            this.refocus(window, cx);
                            cx.notify();
                        }),
                    ),
            );
        }
        if items.len() > shown {
            list = list.child(
                div()
                    .text_size(text::CAPTION)
                    .text_color(theme.secondary)
                    .child(format!("{} more. Keep typing.", items.len() - shown)),
            );
        }
        panel.child(list)
    }

    /// The selected key's macro as text, when the key plays one the
    /// keyboard stores.
    fn selected_macro_text(&self) -> Option<String> {
        let index = macro_index(self.selected_code()?)?;
        let actions = self
            .session
            .as_ref()?
            .macros
            .as_ref()?
            .macros
            .get(usize::from(index))?;
        Some(macros::to_text(actions))
    }

    fn lighting_controls(
        &mut self,
        mut panel: gpui_kit::Stateful<gpui_kit::Div>,
        cx: &mut Context<'_, Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let Some(light) = self
            .session
            .as_ref()
            .and_then(|session| session.lighting.first())
            .copied()
        else {
            return panel;
        };
        panel = panel.child(separator(cx)).child(
            div()
                .text_size(text::CAPTION)
                .text_color(cx.omarchy().secondary)
                .child(light.channel.label()),
        );
        panel = light_row(
            panel,
            "brightness",
            "Brightness",
            light.brightness.to_string(),
            8,
            cx,
            |this, delta| this.adjust_light(delta, LightField::Brightness),
        );
        panel = light_row(
            panel,
            "effect",
            "Effect",
            effect_label(light.channel, light.effect),
            1,
            cx,
            |this, delta| this.adjust_light(delta, LightField::Effect),
        );
        if light.channel.has_speed() {
            panel = light_row(
                panel,
                "speed",
                "Speed",
                light.speed.to_string(),
                8,
                cx,
                |this, delta| this.adjust_light(delta, LightField::Speed),
            );
        }
        if light.channel.has_color() {
            panel = light_row(
                panel,
                "hue",
                "Hue",
                light.hue.to_string(),
                8,
                cx,
                |this, delta| this.adjust_light(delta, LightField::Hue),
            );
        }
        panel
    }

    fn overlay_view(&mut self, cx: &mut Context<'_, Self>) -> Option<impl IntoElement> {
        let theme = cx.omarchy().clone();
        let body = match self.overlay {
            Overlay::None => return None,
            Overlay::Help => self.help_card(cx),
            Overlay::Confirm(action) => self.confirm_card(action, cx),
            Overlay::Macro => self.macro_card(cx)?,
        };
        Some(
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p(space::LG)
                .bg(theme.background.opacity(0.86))
                .child(
                    div()
                        .id("overlay-card")
                        .w(size::HELP)
                        .max_w_full()
                        .max_h_full()
                        .overflow_y_scroll()
                        .child(body),
                ),
        )
    }

    fn help_card(&self, cx: &mut Context<'_, Self>) -> gpui_kit::Div {
        let rows = [
            ("← → ↑ ↓", "move between keys"),
            ("1 – 8", "choose a layer"),
            ("enter", "pick a keycode"),
            ("esc", "close the picker"),
            ("m", "edit macros"),
            ("- =", "brightness"),
            ("[ ]", "lighting effect"),
            ("; '", "hue"),
            ("shift - =", "effect speed"),
            ("ctrl s", "save the keymap"),
            ("r", "look for keyboards again"),
            ("?", "this list"),
            ("q", "quit"),
        ];
        let mut card = div()
            .flex()
            .flex_col()
            .gap(space::SM)
            .p(space::XL)
            .border_1()
            .border_color(cx.omarchy().border)
            .bg(cx.omarchy().background)
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::BOLD)
                    .child("Keys"),
            );
        for (keys, does) in rows {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::MD)
                    .child(div().w(size::KEY_LANE).child(keycap(keys, cx)))
                    .child(does),
            );
        }
        card.child(
            div()
                .text_size(text::CAPTION)
                .text_color(cx.omarchy().secondary)
                .pt(space::SM)
                .child("Enter confirms a reset. Escape cancels it."),
        )
    }

    fn macro_card(&mut self, cx: &mut Context<'_, Self>) -> Option<gpui_kit::Div> {
        let edit = self.macro_edit.clone()?;
        let stored = self.session.as_ref()?.macros.as_ref()?;
        let theme = cx.omarchy().clone();
        // Bytes the buffer would hold with this text in place of the saved
        // macro, so running out of room shows before enter, not after.
        let room = macros::from_text(&edit.text).ok().map(|actions| {
            (
                stored.used_with(edit.index, &actions),
                usize::from(stored.size),
            )
        });
        let count = stored.count();
        let (before, after) = edit.text.split_at(edit.cursor);
        let syntax = [
            ("Hello", "types the text"),
            ("{KC_ENTER}", "taps a key"),
            ("{+KC_LSFT} {-KC_LSFT}", "holds, then releases"),
            ("{KC_LCTL,KC_C}", "presses keys together"),
            ("{250}", "waits 250 ms"),
            ("\\{", "a literal brace"),
        ];
        let mut card = div()
            .flex()
            .flex_col()
            .gap(space::MD)
            .p(space::XL)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(text::TITLE)
                            .font_weight(FontWeight::BOLD)
                            .child(format!("Macro M{}", edit.index)),
                    )
                    .child(
                        div()
                            .text_size(text::CAPTION)
                            .text_color(theme.secondary)
                            .child(format!("{} of {count} · tab for the next", edit.index + 1)),
                    ),
            )
            .child(
                div()
                    .p(space::SM)
                    .border_1()
                    .border_color(theme.accent)
                    .font_family(theme.mono_font.clone())
                    .text_size(text::TITLE)
                    .child(format!("{before}▏{after}")),
            );
        card = card.child(
            div()
                .text_size(text::CAPTION)
                .text_color(match (&edit.error, room) {
                    (Some(_), _) => theme.danger,
                    (None, Some((used, size))) if used > size => theme.danger,
                    _ => theme.secondary,
                })
                .child(match (&edit.error, room) {
                    (Some(error), _) => error.clone(),
                    (None, Some((used, size))) => format!("{used} of {size} bytes"),
                    (None, None) => "Not a macro yet; enter says why.".to_owned(),
                }),
        );
        for (example, does) in syntax {
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::MD)
                    .text_size(text::CAPTION)
                    .child(
                        div()
                            .w(size::KEY_LANE)
                            .flex_shrink_0()
                            .font_family(theme.mono_font.clone())
                            .child(example),
                    )
                    .child(div().text_color(theme.secondary).child(does)),
            );
        }
        Some(
            card.child(
                div()
                    .flex()
                    .gap(space::SM)
                    .child(
                        button("macro-save", "Save", ButtonVariant::Primary, cx).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.save_macro();
                                this.refocus(window, cx);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("macro-close", "Close", ButtonVariant::Secondary, cx).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.close_macro();
                                this.refocus(window, cx);
                                cx.notify();
                            }),
                        ),
                    ),
            ),
        )
    }

    fn confirm_card(&mut self, action: Confirm, cx: &mut Context<'_, Self>) -> gpui_kit::Div {
        let (title, body) = match action {
            Confirm::ResetKeymap => (
                "Reset the keymap?",
                "Every key goes back to the firmware default. This writes to the keyboard.",
            ),
            Confirm::ResetEeprom => (
                "Clear the EEPROM?",
                "Layout options, keymap and macros return to the firmware default, and the keyboard may restart.",
            ),
            Confirm::Bootloader => (
                "Jump to the bootloader?",
                "The keyboard disconnects so it can be flashed. It will not type until it is flashed or reset.",
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap(space::MD)
            .p(space::XL)
            .border_1()
            .border_color(cx.omarchy().danger)
            .bg(cx.omarchy().background)
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .child(body)
            .child(
                div()
                    .flex()
                    .gap(space::SM)
                    .child(
                        button("confirm-yes", "Reset", ButtonVariant::Danger, cx).on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.overlay = Overlay::None;
                                this.commit(action);
                                this.refocus(window, cx);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("confirm-no", "Cancel", ButtonVariant::Secondary, cx).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.overlay = Overlay::None;
                                this.refocus(window, cx);
                                cx.notify();
                            }),
                        ),
                    ),
            )
    }
}

fn light_row(
    panel: gpui_kit::Stateful<gpui_kit::Div>,
    id: &str,
    label: &str,
    value: String,
    step: i16,
    cx: &mut Context<'_, Omakeeb>,
    apply: impl Fn(&mut Omakeeb, i16) + 'static + Clone,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let down = apply.clone();
    panel.child(
        div()
            .flex()
            .items_center()
            .gap(space::SM)
            .child(div().w(rems_width_label()).child(label.to_owned()))
            .child(
                button(
                    SharedString::from(format!("{id}-down")),
                    "−",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    down(this, -step);
                    this.refocus(window, cx);
                    cx.notify();
                })),
            )
            .child(
                div()
                    .flex_1()
                    .font_family(cx.omarchy().mono_font.clone())
                    .child(value),
            )
            .child(
                button(
                    SharedString::from(format!("{id}-up")),
                    "+",
                    ButtonVariant::Secondary,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    apply(this, step);
                    this.refocus(window, cx);
                    cx.notify();
                })),
            ),
    )
}

fn typed_char(key: &str, shift: bool) -> Option<char> {
    if key == "space" {
        return Some(' ');
    }
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || !ch.is_ascii() || ch.is_ascii_control() {
        return None;
    }
    if shift {
        Some(match ch {
            '-' => '_',
            '=' => '+',
            ',' => '<',
            '.' => '>',
            '/' => '?',
            ';' => ':',
            '\'' => '"',
            other if other.is_ascii_lowercase() => other.to_ascii_uppercase(),
            other => other,
        })
    } else {
        Some(ch)
    }
}

/// One line of a macro for the picker. Long macros are cut, not wrapped.
fn macro_preview(text: &str) -> String {
    const LONGEST: usize = 28;
    if text.chars().count() <= LONGEST {
        return text.to_owned();
    }
    let cut: String = text.chars().take(LONGEST - 1).collect();
    format!("{cut}…")
}

fn grant_hid_access() -> bool {
    let Some(script) = setup_script() else {
        return false;
    };
    // Only the udev half: from the window there is no terminal, and a plain
    // run would try to rebuild from wherever the script was installed.
    std::process::Command::new(&script)
        .arg("--root-only")
        .status()
        .is_ok_and(|status| status.success())
}

fn setup_script() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let installed = home.join(".local/lib/omakeeb/setup");
    if installed.is_file() {
        return Some(installed);
    }
    let from_source =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/setup");
    from_source.is_file().then_some(from_source)
}

fn same_devices(left: &[Discovered], right: &[Discovered]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.path == right.path)
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "keymap".to_owned()
    } else {
        out.to_owned()
    }
}

/// How tall the keycode list may grow before it scrolls. `None` beside the
/// board: there it fills the panel's spare height, however tall the window.
/// Under the board the whole stack scrolls, so nothing is left over to fill;
/// the list takes most of the window's height instead, since a fixed height
/// is a sliver in a half or quarter tile.
fn picker_height(stacked: bool, window: &Window) -> Option<gpui_kit::Pixels> {
    if !stacked {
        return None;
    }
    let rem = board::pixels(window.rem_size());
    let height = board::pixels(window.viewport_size().height);
    Some(px(
        (size::PICKER_LIST_MIN.0 * rem).max(height * size::STACKED_LIST_SHARE)
    ))
}

/// Whether the window is too narrow to keep the panel beside the board.
fn stacked(window: &Window) -> bool {
    let rem = board::pixels(window.rem_size());
    board::pixels(window.viewport_size().width) < size::STACK_BELOW.0 * rem
}

/// Pixels left for the board once the bar, the status line, the column's
/// padding and, side by side, the panel have theirs. The two pixels are the
/// dividers either side of the board.
fn board_space(stacked: bool, window: &Window) -> (f32, f32) {
    let rem = board::pixels(window.rem_size());
    let viewport = window.viewport_size();
    let padding = space::LG.0 * 2.0 * rem;
    let chrome = (size::BAR.0 + size::STATUS.0) * rem + padding + 2.0;
    let mut width = board::pixels(viewport.width) - padding - 2.0;
    let mut height = board::pixels(viewport.height) - chrome;
    if stacked {
        height *= size::STACKED_BOARD_SHARE;
    } else {
        width -= size::PANEL.0 * rem;
    }
    (width.max(0.0), height.max(0.0))
}

fn rems_width_label() -> gpui_kit::Rems {
    gpui_kit::Rems(5.5)
}
