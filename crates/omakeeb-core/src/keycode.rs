//! Keycodes the picker can name, and a parser for the ones it cannot list.
//!
//! The numbers are the ones a VIA keyboard already stores. A code the table
//! does not name still round-trips through `0xNNNN`, so a keymap read off a
//! keyboard is never shown as a blank.

use std::sync::OnceLock;

use crate::layout::CustomKey;

/// A section of the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Basic,
    Modifiers,
    Navigation,
    Function,
    Numpad,
    Media,
    Mouse,
    Layer,
    Quantum,
    Lighting,
    Macro,
    Keyboard,
}

impl Group {
    pub const ALL: [Self; 12] = [
        Self::Basic,
        Self::Modifiers,
        Self::Navigation,
        Self::Function,
        Self::Numpad,
        Self::Media,
        Self::Mouse,
        Self::Layer,
        Self::Quantum,
        Self::Lighting,
        Self::Macro,
        Self::Keyboard,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Modifiers => "Modifiers",
            Self::Navigation => "Navigation",
            Self::Function => "Function",
            Self::Numpad => "Numpad",
            Self::Media => "Media",
            Self::Mouse => "Mouse",
            Self::Layer => "Layers",
            Self::Quantum => "QMK",
            Self::Lighting => "Lighting",
            Self::Macro => "Macros",
            Self::Keyboard => "Keyboard",
        }
    }
}

/// One row of the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub code: u16,
    pub name: String,
    pub short: String,
    pub group: Group,
}

struct Entry {
    code: u16,
    name: &'static str,
    short: &'static str,
    group: Group,
}

const QK_MOMENTARY: u16 = 0x5220;
const QK_TOGGLE_LAYER: u16 = 0x5260;
const QK_TO: u16 = 0x5200;
const QK_LAYER_TAP_TOGGLE: u16 = 0x52C0;
const QK_DEF_LAYER: u16 = 0x5240;
const QK_ONE_SHOT_LAYER: u16 = 0x5280;
const QK_PERSISTENT_DEF_LAYER: u16 = 0x52E0;
const QK_LAYER_TAP: u16 = 0x4000;
const QK_MOD_TAP: u16 = 0x2000;
const QK_MODS: u16 = 0x0100;
const QK_MACRO: u16 = 0x7700;
/// `QK_MACRO` through `QK_MACRO_MAX`: QMK reserves 128 macro keycodes.
const MACRO_SLOTS: u16 = 128;
const QK_KB: u16 = 0x7E00;

const MOD_LCTL: u16 = 0x01;
const MOD_LSFT: u16 = 0x02;
const MOD_LALT: u16 = 0x04;
const MOD_LGUI: u16 = 0x08;
const MOD_RIGHT: u16 = 0x10;

/// The macro a keycode plays, if it is `M0` and up.
pub fn macro_index(code: u16) -> Option<u8> {
    (QK_MACRO..QK_MACRO + MACRO_SLOTS)
        .contains(&code)
        .then(|| (code - QK_MACRO) as u8)
}

/// The keycode that plays macro `index`.
pub fn macro_code(index: u8) -> u16 {
    QK_MACRO + u16::from(index)
}

/// The legend drawn on a keycap.
pub fn short_name(code: u16, custom: &[CustomKey]) -> String {
    decode(code, custom).short
}

/// The name shown in the panel and accepted by [`parse`].
pub fn long_name(code: u16, custom: &[CustomKey]) -> String {
    decode(code, custom).long
}

/// Parse a QMK name, a short legend, a call such as `LT(1, KC_SPC)`, or `0xNNNN`.
pub fn parse(text: &str) -> Option<u16> {
    parse_expr(text.trim())
}

/// Items for one picker section. Layer entries cover layers `0..layers`, and
/// macro entries the `macros` the firmware reported.
pub fn items_for(group: Group, custom: &[CustomKey], layers: u8, macros: u8) -> Vec<Item> {
    let mut items = Vec::new();
    if group == Group::Layer {
        let layers = layers.clamp(1, 16);
        for layer in 0..layers {
            for (kind, name) in [
                (QK_MOMENTARY, "MO"),
                (QK_TOGGLE_LAYER, "TG"),
                (QK_TO, "TO"),
                (QK_LAYER_TAP_TOGGLE, "TT"),
                (QK_DEF_LAYER, "DF"),
                (QK_ONE_SHOT_LAYER, "OSL"),
            ] {
                let code = kind | u16::from(layer);
                items.push(Item {
                    code,
                    name: format!("{name}({layer})"),
                    short: format!("{name}({layer})"),
                    group,
                });
            }
        }
        return items;
    }
    if group == Group::Keyboard {
        for key in custom {
            items.push(Item {
                code: QK_KB + u16::from(key.index),
                name: key.name.clone(),
                short: key.short.clone(),
                group,
            });
        }
        return items;
    }
    if group == Group::Macro {
        for index in 0..u16::from(macros).min(MACRO_SLOTS) {
            items.push(Item {
                code: QK_MACRO + index,
                name: format!("M{index}"),
                short: format!("M{index}"),
                group,
            });
        }
        return items;
    }
    for entry in table() {
        if entry.group == group {
            items.push(Item {
                code: entry.code,
                name: entry.name.to_owned(),
                short: entry.short.to_owned(),
                group,
            });
        }
    }
    items
}

struct Names {
    long: String,
    short: String,
}

fn decode(code: u16, custom: &[CustomKey]) -> Names {
    if let Some(entry) = table().iter().find(|entry| entry.code == code) {
        return Names {
            long: entry.name.to_owned(),
            short: entry.short.to_owned(),
        };
    }
    if let Some(key) = custom
        .iter()
        .find(|key| QK_KB + u16::from(key.index) == code)
    {
        return Names {
            long: key.name.clone(),
            short: key.short.clone(),
        };
    }
    if let Some(names) = decode_layer(code) {
        return names;
    }
    if (QK_MOD_TAP..0x4000).contains(&code) {
        let mods = (code >> 8) & 0x1F;
        let tap = code & 0xFF;
        if mods != 0 && tap != 0 {
            let inner = decode(tap, custom);
            return Names {
                long: format_mod_tap(mods, &inner.long),
                short: format!("{}/{}", mod_short(mods), inner.short),
            };
        }
    }
    if (QK_MODS..QK_MOD_TAP).contains(&code) {
        let mods = (code >> 8) & 0x1F;
        let tap = code & 0xFF;
        if mods != 0 {
            let inner = decode(tap, custom);
            return Names {
                long: format_mod_hold(mods, &inner.long),
                short: format!("{}{}", mod_short(mods), inner.short),
            };
        }
    }
    if (QK_MACRO..QK_MACRO + MACRO_SLOTS).contains(&code) {
        let index = code - QK_MACRO;
        return Names {
            long: format!("M{index}"),
            short: format!("M{index}"),
        };
    }
    if (QK_KB..QK_KB + 32).contains(&code) {
        let index = code - QK_KB;
        return Names {
            long: format!("KB{index}"),
            short: format!("KB{index}"),
        };
    }
    Names {
        long: format!("0x{code:04X}"),
        short: format!("0x{code:04X}"),
    }
}

fn decode_layer(code: u16) -> Option<Names> {
    let (base, name) = if (QK_MOMENTARY..QK_MOMENTARY + 32).contains(&code) {
        (QK_MOMENTARY, "MO")
    } else if (QK_TOGGLE_LAYER..QK_TOGGLE_LAYER + 32).contains(&code) {
        (QK_TOGGLE_LAYER, "TG")
    } else if (QK_TO..QK_TO + 32).contains(&code) {
        (QK_TO, "TO")
    } else if (QK_LAYER_TAP_TOGGLE..QK_LAYER_TAP_TOGGLE + 32).contains(&code) {
        (QK_LAYER_TAP_TOGGLE, "TT")
    } else if (QK_DEF_LAYER..QK_DEF_LAYER + 32).contains(&code) {
        (QK_DEF_LAYER, "DF")
    } else if (QK_ONE_SHOT_LAYER..QK_ONE_SHOT_LAYER + 32).contains(&code) {
        (QK_ONE_SHOT_LAYER, "OSL")
    } else if (QK_PERSISTENT_DEF_LAYER..QK_PERSISTENT_DEF_LAYER + 32).contains(&code) {
        (QK_PERSISTENT_DEF_LAYER, "PDF")
    } else if (QK_LAYER_TAP..QK_LAYER_TAP + 0x1000).contains(&code) {
        let layer = ((code >> 8) & 0x0F) as u8;
        let tap = code & 0xFF;
        let inner = decode(tap, &[]);
        let text = format!("LT({layer}, {})", inner.long);
        let short = format!("L{layer}·{}", inner.short);
        return Some(Names { long: text, short });
    } else {
        return None;
    };
    let layer = code - base;
    let text = format!("{name}({layer})");
    Some(Names {
        long: text.clone(),
        short: text,
    })
}

fn format_mod_hold(mods: u16, inner: &str) -> String {
    match mods {
        0x0F => format!("HYPR({inner})"),
        0x07 => format!("MEH({inner})"),
        bits => nest_mods(bits, inner),
    }
}

fn format_mod_tap(mods: u16, inner: &str) -> String {
    let single = matches!(mods & !MOD_RIGHT, MOD_LCTL | MOD_LSFT | MOD_LALT | MOD_LGUI);
    if single {
        let name = mod_function(mods);
        return format!("{name}_T({inner})");
    }
    format!("MT({}, {inner})", mod_arg(mods))
}

fn nest_mods(mods: u16, inner: &str) -> String {
    let mut text = inner.to_owned();
    let right = mods & MOD_RIGHT != 0;
    for (bit, left, right_name) in [
        (MOD_LGUI, "LGUI", "RGUI"),
        (MOD_LALT, "LALT", "RALT"),
        (MOD_LSFT, "LSFT", "RSFT"),
        (MOD_LCTL, "LCTL", "RCTL"),
    ] {
        if mods & bit != 0 {
            let name = if right { right_name } else { left };
            text = format!("{name}({text})");
        }
    }
    text
}

fn mod_function(mods: u16) -> &'static str {
    match mods {
        MOD_LCTL => "LCTL",
        MOD_LSFT => "LSFT",
        MOD_LALT => "LALT",
        MOD_LGUI => "LGUI",
        x if x == MOD_RIGHT | MOD_LCTL => "RCTL",
        x if x == MOD_RIGHT | MOD_LSFT => "RSFT",
        x if x == MOD_RIGHT | MOD_LALT => "RALT",
        x if x == MOD_RIGHT | MOD_LGUI => "RGUI",
        _ => "MT",
    }
}

fn mod_arg(mods: u16) -> String {
    let right = mods & MOD_RIGHT != 0;
    let mut names = Vec::new();
    for (bit, left, right_name) in [
        (MOD_LCTL, "MOD_LCTL", "MOD_RCTL"),
        (MOD_LSFT, "MOD_LSFT", "MOD_RSFT"),
        (MOD_LALT, "MOD_LALT", "MOD_RALT"),
        (MOD_LGUI, "MOD_LGUI", "MOD_RGUI"),
    ] {
        if mods & bit != 0 {
            names.push(if right { right_name } else { left });
        }
    }
    names.join("|")
}

fn mod_short(mods: u16) -> String {
    match mods {
        0x0F => "H".to_owned(),
        0x07 => "M".to_owned(),
        bits => {
            let right = bits & MOD_RIGHT != 0;
            let mut text = String::new();
            if right {
                text.push('R');
            }
            for (bit, glyph) in [
                (MOD_LCTL, 'C'),
                (MOD_LSFT, 'S'),
                (MOD_LALT, 'A'),
                (MOD_LGUI, 'G'),
            ] {
                if bits & bit != 0 {
                    text.push(glyph);
                }
            }
            text
        }
    }
}

fn parse_expr(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))
        && let Ok(code) = u16::from_str_radix(hex, 16)
    {
        return Some(code);
    }
    if let Some(code) = lookup(text) {
        return Some(code);
    }
    let (name, args) = split_call(text)?;
    let name = name.to_ascii_uppercase();
    match name.as_str() {
        "MO" => layer_code(QK_MOMENTARY, &args),
        "TG" => layer_code(QK_TOGGLE_LAYER, &args),
        "TO" => layer_code(QK_TO, &args),
        "TT" => layer_code(QK_LAYER_TAP_TOGGLE, &args),
        "DF" => layer_code(QK_DEF_LAYER, &args),
        "OSL" => layer_code(QK_ONE_SHOT_LAYER, &args),
        "PDF" => layer_code(QK_PERSISTENT_DEF_LAYER, &args),
        "LT" => {
            if args.len() != 2 {
                return None;
            }
            let layer = parse_layer(&args[0])?;
            let tap = parse_expr(&args[1])?;
            Some(QK_LAYER_TAP | (u16::from(layer) << 8) | (tap & 0xFF))
        }
        "LCTL" | "C" => wrap_mod(MOD_LCTL, &args),
        "LSFT" | "S" => wrap_mod(MOD_LSFT, &args),
        "LALT" | "A" => wrap_mod(MOD_LALT, &args),
        "LGUI" | "G" | "LCMD" | "LWIN" => wrap_mod(MOD_LGUI, &args),
        "RCTL" => wrap_mod(MOD_RIGHT | MOD_LCTL, &args),
        "RSFT" => wrap_mod(MOD_RIGHT | MOD_LSFT, &args),
        "RALT" | "ALGR" => wrap_mod(MOD_RIGHT | MOD_LALT, &args),
        "RGUI" | "RCMD" | "RWIN" => wrap_mod(MOD_RIGHT | MOD_LGUI, &args),
        "HYPR" => wrap_mod(0x0F, &args),
        "MEH" => wrap_mod(0x07, &args),
        "LCTL_T" => mod_tap(MOD_LCTL, &args),
        "LSFT_T" => mod_tap(MOD_LSFT, &args),
        "LALT_T" => mod_tap(MOD_LALT, &args),
        "LGUI_T" => mod_tap(MOD_LGUI, &args),
        "RCTL_T" => mod_tap(MOD_RIGHT | MOD_LCTL, &args),
        "RSFT_T" => mod_tap(MOD_RIGHT | MOD_LSFT, &args),
        "RALT_T" => mod_tap(MOD_RIGHT | MOD_LALT, &args),
        "RGUI_T" => mod_tap(MOD_RIGHT | MOD_LGUI, &args),
        "MT" => {
            if args.len() != 2 {
                return None;
            }
            let mods = parse_mods(&args[0])?;
            mod_tap(mods, std::slice::from_ref(&args[1]))
        }
        "M" if args.len() == 1 => {
            let index = args[0].trim().parse::<u16>().ok()?;
            (index < MACRO_SLOTS).then_some(QK_MACRO + index)
        }
        _ => None,
    }
}

fn layer_code(base: u16, args: &[String]) -> Option<u16> {
    if args.len() != 1 {
        return None;
    }
    let layer = parse_layer(&args[0])?;
    Some(base | u16::from(layer))
}

fn parse_layer(text: &str) -> Option<u8> {
    let layer: u8 = text.trim().parse().ok()?;
    (layer < 32).then_some(layer)
}

fn wrap_mod(mods: u16, args: &[String]) -> Option<u16> {
    if args.len() != 1 {
        return None;
    }
    let inner = parse_expr(&args[0])?;
    if (QK_MODS..QK_MOD_TAP).contains(&inner) {
        let combined = ((inner >> 8) & 0x1F) | mods;
        return Some(QK_MODS | (combined << 8) | (inner & 0xFF));
    }
    Some(QK_MODS | (mods << 8) | (inner & 0xFF))
}

fn mod_tap(mods: u16, args: &[String]) -> Option<u16> {
    if args.len() != 1 {
        return None;
    }
    let tap = parse_expr(&args[0])? & 0xFF;
    (tap != 0).then_some(QK_MOD_TAP | (mods << 8) | tap)
}

fn parse_mods(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.contains('|') {
        let mut bits = 0;
        for part in text.split('|') {
            bits |= parse_one_mod(part.trim())?;
        }
        return Some(bits);
    }
    parse_one_mod(text)
}

fn parse_one_mod(text: &str) -> Option<u16> {
    if let Some(code) = lookup(text)
        && let Some(bits) = mod_bits_from_modifier_key(code)
    {
        return Some(bits);
    }
    parse_mod_name(text)
}

fn parse_mod_name(text: &str) -> Option<u16> {
    match text.to_ascii_uppercase().as_str() {
        "MOD_LCTL" | "LCTL" => Some(MOD_LCTL),
        "MOD_LSFT" | "LSFT" => Some(MOD_LSFT),
        "MOD_LALT" | "LALT" => Some(MOD_LALT),
        "MOD_LGUI" | "LGUI" => Some(MOD_LGUI),
        "MOD_RCTL" | "RCTL" => Some(MOD_RIGHT | MOD_LCTL),
        "MOD_RSFT" | "RSFT" => Some(MOD_RIGHT | MOD_LSFT),
        "MOD_RALT" | "RALT" => Some(MOD_RIGHT | MOD_LALT),
        "MOD_RGUI" | "RGUI" => Some(MOD_RIGHT | MOD_LGUI),
        "MOD_HYPR" | "HYPR" => Some(0x0F),
        "MOD_MEH" | "MEH" => Some(0x07),
        _ => None,
    }
}

fn mod_bits_from_modifier_key(code: u16) -> Option<u16> {
    match code {
        0x00E0 => Some(MOD_LCTL),
        0x00E1 => Some(MOD_LSFT),
        0x00E2 => Some(MOD_LALT),
        0x00E3 => Some(MOD_LGUI),
        0x00E4 => Some(MOD_RIGHT | MOD_LCTL),
        0x00E5 => Some(MOD_RIGHT | MOD_LSFT),
        0x00E6 => Some(MOD_RIGHT | MOD_LALT),
        0x00E7 => Some(MOD_RIGHT | MOD_LGUI),
        _ => None,
    }
}

fn split_call(text: &str) -> Option<(String, Vec<String>)> {
    let open = text.find('(')?;
    if !text.ends_with(')') {
        return None;
    }
    let name = text[..open].trim().to_owned();
    let inner = &text[open + 1..text.len() - 1];
    if name.is_empty() {
        return None;
    }
    let mut args = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(inner[start..index].trim().to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() || !args.is_empty() {
        args.push(tail.to_owned());
    }
    Some((name, args))
}

fn lookup(text: &str) -> Option<u16> {
    let folded = text.to_ascii_uppercase();
    if let Some(code) = aliases().get(folded.as_str()).copied() {
        return Some(code);
    }
    table()
        .iter()
        .find(|entry| {
            entry.name.eq_ignore_ascii_case(text) || entry.short.eq_ignore_ascii_case(text)
        })
        .map(|entry| entry.code)
}

fn aliases() -> &'static std::collections::HashMap<&'static str, u16> {
    static ALIASES: OnceLock<std::collections::HashMap<&'static str, u16>> = OnceLock::new();
    ALIASES.get_or_init(|| {
        let pairs = [
            ("KC_TRNS", 0x0001_u16),
            ("_______", 0x0001),
            ("TRNS", 0x0001),
            ("XXXXXXX", 0x0000),
            ("NO", 0x0000),
            ("KC_ENT", 0x0028),
            ("KC_ESC", 0x0029),
            ("KC_BSPC", 0x002A),
            ("KC_SPC", 0x002C),
            ("KC_MINS", 0x002D),
            ("KC_EQL", 0x002E),
            ("KC_LBRC", 0x002F),
            ("KC_RBRC", 0x0030),
            ("KC_BSLS", 0x0031),
            ("KC_SCLN", 0x0033),
            ("KC_QUOT", 0x0034),
            ("KC_GRV", 0x0035),
            ("KC_COMM", 0x0036),
            ("KC_SLSH", 0x0038),
            ("KC_CAPS", 0x0039),
            ("KC_PSCR", 0x0046),
            ("KC_INS", 0x0049),
            ("KC_PGUP", 0x004B),
            ("KC_DEL", 0x004C),
            ("KC_PGDN", 0x004E),
            ("KC_RGHT", 0x004F),
            ("KC_LCTL", 0x00E0),
            ("KC_LSFT", 0x00E1),
            ("KC_LALT", 0x00E2),
            ("KC_LGUI", 0x00E3),
            ("KC_RCTL", 0x00E4),
            ("KC_RSFT", 0x00E5),
            ("KC_RALT", 0x00E6),
            ("KC_RGUI", 0x00E7),
            ("KC_MUTE", 0x00A8),
            ("KC_VOLU", 0x00A9),
            ("KC_VOLD", 0x00AA),
            ("KC_MNXT", 0x00AB),
            ("KC_MPRV", 0x00AC),
            ("KC_MPLY", 0x00AE),
            ("QK_BOOT", 0x7C00),
            ("QK_GESC", 0x7C16),
            ("CW_TOGG", 0x7C73),
            ("QK_REP", 0x7C79),
        ];
        pairs.into_iter().collect()
    })
}

fn table() -> &'static [Entry] {
    const fn e(code: u16, name: &'static str, short: &'static str, group: Group) -> Entry {
        Entry {
            code,
            name,
            short,
            group,
        }
    }
    const T: &[Entry] = &[
        e(0x0000, "KC_NO", "·", Group::Basic),
        e(0x0001, "KC_TRANSPARENT", "▽", Group::Basic),
        e(0x0004, "KC_A", "A", Group::Basic),
        e(0x0005, "KC_B", "B", Group::Basic),
        e(0x0006, "KC_C", "C", Group::Basic),
        e(0x0007, "KC_D", "D", Group::Basic),
        e(0x0008, "KC_E", "E", Group::Basic),
        e(0x0009, "KC_F", "F", Group::Basic),
        e(0x000A, "KC_G", "G", Group::Basic),
        e(0x000B, "KC_H", "H", Group::Basic),
        e(0x000C, "KC_I", "I", Group::Basic),
        e(0x000D, "KC_J", "J", Group::Basic),
        e(0x000E, "KC_K", "K", Group::Basic),
        e(0x000F, "KC_L", "L", Group::Basic),
        e(0x0010, "KC_M", "M", Group::Basic),
        e(0x0011, "KC_N", "N", Group::Basic),
        e(0x0012, "KC_O", "O", Group::Basic),
        e(0x0013, "KC_P", "P", Group::Basic),
        e(0x0014, "KC_Q", "Q", Group::Basic),
        e(0x0015, "KC_R", "R", Group::Basic),
        e(0x0016, "KC_S", "S", Group::Basic),
        e(0x0017, "KC_T", "T", Group::Basic),
        e(0x0018, "KC_U", "U", Group::Basic),
        e(0x0019, "KC_V", "V", Group::Basic),
        e(0x001A, "KC_W", "W", Group::Basic),
        e(0x001B, "KC_X", "X", Group::Basic),
        e(0x001C, "KC_Y", "Y", Group::Basic),
        e(0x001D, "KC_Z", "Z", Group::Basic),
        e(0x001E, "KC_1", "1", Group::Basic),
        e(0x001F, "KC_2", "2", Group::Basic),
        e(0x0020, "KC_3", "3", Group::Basic),
        e(0x0021, "KC_4", "4", Group::Basic),
        e(0x0022, "KC_5", "5", Group::Basic),
        e(0x0023, "KC_6", "6", Group::Basic),
        e(0x0024, "KC_7", "7", Group::Basic),
        e(0x0025, "KC_8", "8", Group::Basic),
        e(0x0026, "KC_9", "9", Group::Basic),
        e(0x0027, "KC_0", "0", Group::Basic),
        e(0x0028, "KC_ENTER", "Enter", Group::Basic),
        e(0x0029, "KC_ESCAPE", "Esc", Group::Basic),
        e(0x002A, "KC_BACKSPACE", "Bspc", Group::Basic),
        e(0x002B, "KC_TAB", "Tab", Group::Basic),
        e(0x002C, "KC_SPACE", "Space", Group::Basic),
        e(0x002D, "KC_MINUS", "-", Group::Basic),
        e(0x002E, "KC_EQUAL", "=", Group::Basic),
        e(0x002F, "KC_LEFT_BRACKET", "[", Group::Basic),
        e(0x0030, "KC_RIGHT_BRACKET", "]", Group::Basic),
        e(0x0031, "KC_BACKSLASH", "\\", Group::Basic),
        e(0x0032, "KC_NONUS_HASH", "#", Group::Basic),
        e(0x0033, "KC_SEMICOLON", ";", Group::Basic),
        e(0x0034, "KC_QUOTE", "'", Group::Basic),
        e(0x0035, "KC_GRAVE", "`", Group::Basic),
        e(0x0036, "KC_COMMA", ",", Group::Basic),
        e(0x0037, "KC_DOT", ".", Group::Basic),
        e(0x0038, "KC_SLASH", "/", Group::Basic),
        e(0x0039, "KC_CAPS_LOCK", "Caps", Group::Basic),
        e(0x0064, "KC_NONUS_BACKSLASH", "\\", Group::Basic),
        e(0x0065, "KC_APPLICATION", "Menu", Group::Basic),
        e(0x00E0, "KC_LEFT_CTRL", "Ctrl", Group::Modifiers),
        e(0x00E1, "KC_LEFT_SHIFT", "Shift", Group::Modifiers),
        e(0x00E2, "KC_LEFT_ALT", "Alt", Group::Modifiers),
        e(0x00E3, "KC_LEFT_GUI", "Gui", Group::Modifiers),
        e(0x00E4, "KC_RIGHT_CTRL", "RCtrl", Group::Modifiers),
        e(0x00E5, "KC_RIGHT_SHIFT", "RShift", Group::Modifiers),
        e(0x00E6, "KC_RIGHT_ALT", "RAlt", Group::Modifiers),
        e(0x00E7, "KC_RIGHT_GUI", "RGui", Group::Modifiers),
        e(0x004F, "KC_RIGHT", "Right", Group::Navigation),
        e(0x0050, "KC_LEFT", "Left", Group::Navigation),
        e(0x0051, "KC_DOWN", "Down", Group::Navigation),
        e(0x0052, "KC_UP", "Up", Group::Navigation),
        e(0x0049, "KC_INSERT", "Ins", Group::Navigation),
        e(0x004A, "KC_HOME", "Home", Group::Navigation),
        e(0x004B, "KC_PAGE_UP", "PgUp", Group::Navigation),
        e(0x004C, "KC_DELETE", "Del", Group::Navigation),
        e(0x004D, "KC_END", "End", Group::Navigation),
        e(0x004E, "KC_PAGE_DOWN", "PgDn", Group::Navigation),
        e(0x0046, "KC_PRINT_SCREEN", "PrtSc", Group::Navigation),
        e(0x0047, "KC_SCROLL_LOCK", "Scrl", Group::Navigation),
        e(0x0048, "KC_PAUSE", "Pause", Group::Navigation),
        e(0x003A, "KC_F1", "F1", Group::Function),
        e(0x003B, "KC_F2", "F2", Group::Function),
        e(0x003C, "KC_F3", "F3", Group::Function),
        e(0x003D, "KC_F4", "F4", Group::Function),
        e(0x003E, "KC_F5", "F5", Group::Function),
        e(0x003F, "KC_F6", "F6", Group::Function),
        e(0x0040, "KC_F7", "F7", Group::Function),
        e(0x0041, "KC_F8", "F8", Group::Function),
        e(0x0042, "KC_F9", "F9", Group::Function),
        e(0x0043, "KC_F10", "F10", Group::Function),
        e(0x0044, "KC_F11", "F11", Group::Function),
        e(0x0045, "KC_F12", "F12", Group::Function),
        e(0x0068, "KC_F13", "F13", Group::Function),
        e(0x0069, "KC_F14", "F14", Group::Function),
        e(0x006A, "KC_F15", "F15", Group::Function),
        e(0x006B, "KC_F16", "F16", Group::Function),
        e(0x006C, "KC_F17", "F17", Group::Function),
        e(0x006D, "KC_F18", "F18", Group::Function),
        e(0x006E, "KC_F19", "F19", Group::Function),
        e(0x006F, "KC_F20", "F20", Group::Function),
        e(0x0070, "KC_F21", "F21", Group::Function),
        e(0x0071, "KC_F22", "F22", Group::Function),
        e(0x0072, "KC_F23", "F23", Group::Function),
        e(0x0073, "KC_F24", "F24", Group::Function),
        e(0x0053, "KC_NUM_LOCK", "Num", Group::Numpad),
        e(0x0054, "KC_KP_SLASH", "P/", Group::Numpad),
        e(0x0055, "KC_KP_ASTERISK", "P*", Group::Numpad),
        e(0x0056, "KC_KP_MINUS", "P-", Group::Numpad),
        e(0x0057, "KC_KP_PLUS", "P+", Group::Numpad),
        e(0x0058, "KC_KP_ENTER", "PEnt", Group::Numpad),
        e(0x0059, "KC_KP_1", "P1", Group::Numpad),
        e(0x005A, "KC_KP_2", "P2", Group::Numpad),
        e(0x005B, "KC_KP_3", "P3", Group::Numpad),
        e(0x005C, "KC_KP_4", "P4", Group::Numpad),
        e(0x005D, "KC_KP_5", "P5", Group::Numpad),
        e(0x005E, "KC_KP_6", "P6", Group::Numpad),
        e(0x005F, "KC_KP_7", "P7", Group::Numpad),
        e(0x0060, "KC_KP_8", "P8", Group::Numpad),
        e(0x0061, "KC_KP_9", "P9", Group::Numpad),
        e(0x0062, "KC_KP_0", "P0", Group::Numpad),
        e(0x0063, "KC_KP_DOT", "P.", Group::Numpad),
        e(0x0067, "KC_KP_EQUAL", "P=", Group::Numpad),
        e(0x00A8, "KC_AUDIO_MUTE", "Mute", Group::Media),
        e(0x00A9, "KC_AUDIO_VOL_UP", "Vol+", Group::Media),
        e(0x00AA, "KC_AUDIO_VOL_DOWN", "Vol-", Group::Media),
        e(0x00AB, "KC_MEDIA_NEXT_TRACK", "Next", Group::Media),
        e(0x00AC, "KC_MEDIA_PREV_TRACK", "Prev", Group::Media),
        e(0x00AD, "KC_MEDIA_STOP", "Stop", Group::Media),
        e(0x00AE, "KC_MEDIA_PLAY_PAUSE", "Play", Group::Media),
        e(0x00B0, "KC_MEDIA_EJECT", "Eject", Group::Media),
        e(0x00B2, "KC_CALCULATOR", "Calc", Group::Media),
        e(0x00BD, "KC_BRIGHTNESS_UP", "Bri+", Group::Media),
        e(0x00BE, "KC_BRIGHTNESS_DOWN", "Bri-", Group::Media),
        e(0x00A5, "KC_SYSTEM_POWER", "Power", Group::Media),
        e(0x00A6, "KC_SYSTEM_SLEEP", "Sleep", Group::Media),
        e(0x00A7, "KC_SYSTEM_WAKE", "Wake", Group::Media),
        e(0x00CD, "MS_UP", "MsUp", Group::Mouse),
        e(0x00CE, "MS_DOWN", "MsDn", Group::Mouse),
        e(0x00CF, "MS_LEFT", "MsLf", Group::Mouse),
        e(0x00D0, "MS_RGHT", "MsRt", Group::Mouse),
        e(0x00D1, "MS_BTN1", "Ms1", Group::Mouse),
        e(0x00D2, "MS_BTN2", "Ms2", Group::Mouse),
        e(0x00D3, "MS_BTN3", "Ms3", Group::Mouse),
        e(0x00D9, "MS_WHLU", "WhUp", Group::Mouse),
        e(0x00DA, "MS_WHLD", "WhDn", Group::Mouse),
        e(0x00DB, "MS_WHLL", "WhLf", Group::Mouse),
        e(0x00DC, "MS_WHLR", "WhRt", Group::Mouse),
        e(0x7C00, "QK_BOOTLOADER", "Boot", Group::Quantum),
        e(0x7C01, "QK_REBOOT", "Reboot", Group::Quantum),
        e(0x7C03, "QK_CLEAR_EEPROM", "EEClr", Group::Quantum),
        e(0x7C16, "QK_GRAVE_ESCAPE", "GEsc", Group::Quantum),
        e(0x7C73, "QK_CAPS_WORD_TOGGLE", "CapsW", Group::Quantum),
        e(0x7C79, "QK_REPEAT_KEY", "Rep", Group::Quantum),
        e(0x7C7A, "QK_ALT_REPEAT_KEY", "ARep", Group::Quantum),
        e(0x7C7B, "QK_LAYER_LOCK", "LLock", Group::Quantum),
        e(0x7800, "QK_BACKLIGHT_ON", "BLOn", Group::Lighting),
        e(0x7801, "QK_BACKLIGHT_OFF", "BLOff", Group::Lighting),
        e(0x7802, "QK_BACKLIGHT_TOGGLE", "BLTog", Group::Lighting),
        e(0x7803, "QK_BACKLIGHT_DOWN", "BL-", Group::Lighting),
        e(0x7804, "QK_BACKLIGHT_UP", "BL+", Group::Lighting),
        e(0x7820, "QK_UNDERGLOW_TOGGLE", "UGTog", Group::Lighting),
        e(0x7821, "QK_UNDERGLOW_MODE_NEXT", "UG+", Group::Lighting),
        e(0x7822, "QK_UNDERGLOW_MODE_PREVIOUS", "UG-", Group::Lighting),
        e(0x7823, "QK_UNDERGLOW_HUE_UP", "UGHue+", Group::Lighting),
        e(0x7827, "QK_UNDERGLOW_VALUE_UP", "UGVal+", Group::Lighting),
        e(0x7828, "QK_UNDERGLOW_VALUE_DOWN", "UGVal-", Group::Lighting),
        e(0x7840, "QK_RGB_MATRIX_ON", "RMOn", Group::Lighting),
        e(0x7841, "QK_RGB_MATRIX_OFF", "RMOff", Group::Lighting),
        e(0x7842, "QK_RGB_MATRIX_TOGGLE", "RMTog", Group::Lighting),
        e(0x7843, "QK_RGB_MATRIX_MODE_NEXT", "RM+", Group::Lighting),
        e(
            0x7844,
            "QK_RGB_MATRIX_MODE_PREVIOUS",
            "RM-",
            Group::Lighting,
        ),
        e(0x7845, "QK_RGB_MATRIX_HUE_UP", "RMHue+", Group::Lighting),
        e(0x7849, "QK_RGB_MATRIX_VALUE_UP", "RMVal+", Group::Lighting),
        e(
            0x784A,
            "QK_RGB_MATRIX_VALUE_DOWN",
            "RMVal-",
            Group::Lighting,
        ),
    ];
    T
}

/// True when the code is a momentary, toggle, or other layer switch, so the
/// board can tint it as a family without competing with the selection.
pub fn is_layer_switch(code: u16) -> bool {
    (0x4000..0x5300).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_keycodes_cover_qmks_range() {
        assert_eq!(macro_index(macro_code(0)), Some(0));
        assert_eq!(macro_index(0x777F), Some(127));
        assert_eq!(macro_index(0x7780), None);
        assert_eq!(parse("M(20)"), Some(0x7714));
        assert_eq!(items_for(Group::Macro, &[], 4, 3).len(), 3);
        assert!(items_for(Group::Macro, &[], 4, 0).is_empty());
    }

    #[test]
    fn basic_legends_and_aliases() {
        assert_eq!(short_name(0x0004, &[]), "A");
        assert_eq!(long_name(0x0029, &[]), "KC_ESCAPE");
        assert_eq!(short_name(0x0001, &[]), "▽");
        assert_eq!(parse("KC_A"), Some(0x0004));
        assert_eq!(parse("kc_esc"), Some(0x0029));
        assert_eq!(parse("Esc"), Some(0x0029));
        assert_eq!(parse("0x5221"), Some(0x5221));
    }

    #[test]
    fn layer_and_mod_tap_round_trip() {
        assert_eq!(long_name(0x5221, &[]), "MO(1)");
        assert_eq!(parse("MO(1)"), Some(0x5221));
        assert_eq!(parse("LT(1, KC_SPC)"), Some(0x4000 | (1 << 8) | 0x002C));
        assert_eq!(parse("LSFT_T(KC_A)"), Some(0x2000 | (0x02 << 8) | 0x0004));
        let code = parse("LCTL(LSFT(KC_A))").unwrap();
        assert_eq!(code, 0x0100 | (0x03 << 8) | 0x0004);
        assert_eq!(parse(&long_name(code, &[])).unwrap(), code);
        assert_eq!(parse("HYPR(KC_ESC)"), Some(0x0F00 | 0x0029));
    }

    #[test]
    fn every_listed_name_parses_back() {
        for entry in table() {
            assert_eq!(parse(entry.name), Some(entry.code), "{}", entry.name);
            assert_eq!(long_name(entry.code, &[]), entry.name);
        }
    }
}
