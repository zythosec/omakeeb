//! Dynamic macros: what the firmware stores, and the text a person edits.
//!
//! The firmware keeps every macro in one buffer, each ended by a NUL. Plain
//! bytes are typed as ASCII. `0x01` starts an action: `0x01 0x01 kc` taps a
//! basic keycode, `0x02` holds it, `0x03` releases it, and `0x04` waits for
//! the decimal milliseconds that follow, up to a `|`. That is QMK's
//! `SEND_STRING` encoding, which `dynamic_keymap_macro_send` replays.
//!
//! Anything else is kept byte for byte, so a macro written by another tool,
//! such as Vial's two-byte keycode actions, survives an edit of its neighbour.
//!
//! The text form is VIA's: `Hello{KC_ENTER}` types and taps, `{+KC_LSFT}` and
//! `{-KC_LSFT}` hold and release, `{KC_LCTL,KC_C}` presses a chord, `{250}`
//! waits. `\{` and `\\` are a literal brace and backslash.

use std::fmt::Write as _;

use crate::error::{Error, Result};
use crate::keycode;
use crate::protocol::{self, Report};

const PREFIX: u8 = 0x01;
const TAP: u8 = 0x01;
const DOWN: u8 = 0x02;
const UP: u8 = 0x03;
const DELAY: u8 = 0x04;
/// Vial's tap, down and up with a two-byte keycode. Kept, not edited.
const VIAL_EXTENDED: std::ops::RangeInclusive<u8> = 0x05..=0x07;
const DELAY_END: u8 = b'|';
/// QMK replays a delay from an eight-byte scratch buffer: prefix, code, the
/// digits and the `|`. Five digits is what fits.
const DELAY_MAX: u32 = 99_999;

/// One step of a macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Printable ASCII, typed as it reads.
    Text(String),
    Tap(u8),
    Down(u8),
    Up(u8),
    Delay(u32),
    /// Bytes omakeeb does not edit, written back as they were read. Never
    /// contains a NUL, which would end the macro.
    Raw(Vec<u8>),
}

/// The macros a keyboard stores, and the room it has for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Macros {
    /// Bytes in the firmware's buffer, shared by every macro.
    pub size: u16,
    pub macros: Vec<Vec<Action>>,
}

impl Macros {
    pub fn count(&self) -> u8 {
        self.macros.len() as u8
    }

    /// Bytes the macros take once encoded, terminators included.
    pub fn used(&self) -> usize {
        self.macros
            .iter()
            .map(|actions| encode(actions).len() + 1)
            .sum()
    }

    /// What [`Self::used`] would be with macro `index` replaced, so an editor
    /// can show running out of room before anything is written.
    pub fn used_with(&self, index: u8, actions: &[Action]) -> usize {
        self.macros
            .iter()
            .enumerate()
            .map(|(slot, stored)| {
                let actions = if slot == usize::from(index) {
                    actions
                } else {
                    stored
                };
                encode(actions).len() + 1
            })
            .sum()
    }

    /// Read the whole buffer. `count` and `size` are what the firmware
    /// reported; the protocol has no other way to know them.
    pub fn read_from_device(
        transact: &mut dyn FnMut(Report) -> Result<Report>,
        count: u8,
        size: u16,
    ) -> Result<Self> {
        let total = usize::from(size);
        let mut bytes = vec![0; total];
        let mut offset = 0;
        while offset < total {
            let chunk = (total - offset).min(protocol::KEYMAP_CHUNK) as u8;
            let reply = transact(protocol::macro_get_request(offset as u16, chunk)?)?;
            let data = protocol::macro_bytes(&reply, chunk)?;
            bytes[offset..offset + usize::from(chunk)].copy_from_slice(data);
            offset += usize::from(chunk);
        }
        Ok(Self {
            size,
            macros: parse_buffer(&bytes, count),
        })
    }

    /// Replace one macro and write the buffer. Nothing is written, and the
    /// macros are unchanged, if the result would not fit.
    pub fn set(
        &mut self,
        index: u8,
        actions: Vec<Action>,
        transact: &mut dyn FnMut(Report) -> Result<Report>,
    ) -> Result<()> {
        let slot = usize::from(index);
        if slot >= self.macros.len() {
            return Err(Error::message(format!(
                "M{index} is not one of the keyboard's {} macros",
                self.macros.len()
            )));
        }
        let mut next = self.macros.clone();
        next[slot] = actions;
        let bytes = encode_buffer(&next, usize::from(self.size))?;
        write_buffer(&bytes, transact)?;
        self.macros = next;
        Ok(())
    }
}

/// Split a buffer into `count` macros. A buffer that ends early gives the
/// rest as empty macros, which is what the firmware plays for them.
pub fn parse_buffer(bytes: &[u8], count: u8) -> Vec<Vec<Action>> {
    let mut macros = Vec::with_capacity(usize::from(count));
    let mut rest = bytes;
    for _ in 0..count {
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(rest.len());
        macros.push(parse(&rest[..end]));
        rest = rest.get(end + 1..).unwrap_or(&[]);
    }
    macros
}

/// Every macro, NUL-terminated, padded with NULs to the buffer's size.
pub fn encode_buffer(macros: &[Vec<Action>], size: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(size);
    for actions in macros {
        bytes.extend(encode(actions));
        bytes.push(0);
    }
    if bytes.len() > size {
        return Err(Error::message(format!(
            "the macros need {} bytes and the keyboard has room for {size}",
            bytes.len()
        )));
    }
    bytes.resize(size, 0);
    Ok(bytes)
}

/// Write a whole buffer. The last byte goes non-zero first and back to zero
/// last: QMK will not play a macro while it is set, so a key pressed during
/// the write does not replay half of one.
fn write_buffer(bytes: &[u8], transact: &mut dyn FnMut(Report) -> Result<Report>) -> Result<()> {
    let Some(last) = bytes.len().checked_sub(1) else {
        return Ok(());
    };
    let mut send = |offset: usize, data: &[u8]| -> Result<()> {
        let reply = transact(protocol::macro_set_request(offset as u16, data)?)?;
        protocol::ensure_command(&reply, protocol::MACRO_SET_BUFFER)
    };
    send(last, &[0xFF])?;
    for (index, chunk) in bytes[..last].chunks(protocol::KEYMAP_CHUNK).enumerate() {
        send(index * protocol::KEYMAP_CHUNK, chunk)?;
    }
    send(last, &bytes[last..])
}

/// One macro's bytes, without its terminator.
pub fn parse(bytes: &[u8]) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte != PREFIX {
            if (0x20..=0x7E).contains(&byte) {
                push_text(&mut actions, char::from(byte));
            } else {
                push_raw(&mut actions, &[byte]);
            }
            at += 1;
            continue;
        }
        let code = bytes.get(at + 1).copied();
        let key = bytes.get(at + 2).copied();
        match (code, key) {
            (Some(TAP), Some(key)) => actions.push(Action::Tap(key)),
            (Some(DOWN), Some(key)) => actions.push(Action::Down(key)),
            (Some(UP), Some(key)) => actions.push(Action::Up(key)),
            (Some(DELAY), _) => {
                let digits = &bytes[at + 2..];
                let end = digits.iter().position(|byte| *byte == DELAY_END);
                let delay = end.and_then(|end| {
                    std::str::from_utf8(&digits[..end])
                        .ok()
                        .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
                        .and_then(|text| text.parse::<u32>().ok())
                        .map(|ms| (ms, end))
                });
                if let Some((ms, end)) = delay {
                    actions.push(Action::Delay(ms));
                    at += 2 + end + 1;
                } else {
                    push_raw(&mut actions, &bytes[at..]);
                    at = bytes.len();
                }
                continue;
            }
            (Some(code), _) if VIAL_EXTENDED.contains(&code) && at + 4 <= bytes.len() => {
                push_raw(&mut actions, &bytes[at..at + 4]);
                at += 4;
                continue;
            }
            _ => {
                // An action this does not know how long to read: keep the
                // rest of the macro as it is rather than guess where it ends.
                push_raw(&mut actions, &bytes[at..]);
                at = bytes.len();
                continue;
            }
        }
        at += 3;
    }
    actions
}

/// One macro's bytes, without its terminator.
pub fn encode(actions: &[Action]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for action in actions {
        match action {
            Action::Text(text) => bytes.extend(text.bytes()),
            Action::Tap(key) => bytes.extend([PREFIX, TAP, *key]),
            Action::Down(key) => bytes.extend([PREFIX, DOWN, *key]),
            Action::Up(key) => bytes.extend([PREFIX, UP, *key]),
            Action::Delay(ms) => {
                bytes.extend([PREFIX, DELAY]);
                bytes.extend(ms.to_string().bytes());
                bytes.push(DELAY_END);
            }
            Action::Raw(raw) => bytes.extend(raw),
        }
    }
    bytes
}

/// The text a person edits.
pub fn to_text(actions: &[Action]) -> String {
    let mut text = String::new();
    let mut at = 0;
    while at < actions.len() {
        if let Some(keys) = chord_at(&actions[at..]) {
            let names: Vec<String> = keys.iter().map(|key| key_name(*key)).collect();
            text.push('{');
            text.push_str(&names.join(","));
            text.push('}');
            at += keys.len() * 2;
            continue;
        }
        match &actions[at] {
            Action::Text(typed) => {
                for ch in typed.chars() {
                    if ch == '{' || ch == '\\' {
                        text.push('\\');
                    }
                    text.push(ch);
                }
            }
            // Writing to a String cannot fail.
            Action::Tap(key) => {
                let _ = write!(text, "{{{}}}", key_name(*key));
            }
            Action::Down(key) => {
                let _ = write!(text, "{{+{}}}", key_name(*key));
            }
            Action::Up(key) => {
                let _ = write!(text, "{{-{}}}", key_name(*key));
            }
            Action::Delay(ms) => {
                let _ = write!(text, "{{{ms}}}");
            }
            Action::Raw(raw) => {
                text.push_str("{raw:");
                for byte in raw {
                    let _ = write!(text, "{byte:02X}");
                }
                text.push('}');
            }
        }
        at += 1;
    }
    text
}

/// Parse the text a person typed. The error names the part that is wrong.
pub fn from_text(text: &str) -> Result<Vec<Action>> {
    let mut actions = Vec::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let escaped = chars
                    .next()
                    .ok_or_else(|| Error::message("a \\ at the end needs a character after it"))?;
                push_text(&mut actions, typable(escaped)?);
            }
            '{' => {
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(next) => inner.push(next),
                        None => return Err(Error::message(format!("{{{inner} is missing its }}"))),
                    }
                }
                braced(inner.trim(), &mut actions)?;
            }
            other => push_text(&mut actions, typable(other)?),
        }
    }
    Ok(actions)
}

fn braced(inner: &str, actions: &mut Vec<Action>) -> Result<()> {
    if inner.is_empty() {
        return Err(Error::message("{} is empty; put a key or a delay in it"));
    }
    if let Some(hex) = inner.strip_prefix("raw:") {
        let raw = parse_hex(hex.trim())?;
        push_raw(actions, &raw);
        return Ok(());
    }
    if inner.bytes().all(|byte| byte.is_ascii_digit()) {
        let ms: u32 = inner
            .parse()
            .ok()
            .filter(|ms| *ms <= DELAY_MAX)
            .ok_or_else(|| Error::message(format!("a delay is at most {DELAY_MAX} ms")))?;
        actions.push(Action::Delay(ms));
        return Ok(());
    }
    if let Some(name) = inner.strip_prefix('+') {
        actions.push(Action::Down(basic_key(name)?));
        return Ok(());
    }
    if let Some(name) = inner.strip_prefix('-') {
        actions.push(Action::Up(basic_key(name)?));
        return Ok(());
    }
    if inner.contains(',') {
        let keys = inner
            .split(',')
            .map(basic_key)
            .collect::<Result<Vec<u8>>>()?;
        actions.extend(keys.iter().map(|key| Action::Down(*key)));
        actions.extend(keys.iter().rev().map(|key| Action::Up(*key)));
        return Ok(());
    }
    actions.push(Action::Tap(basic_key(inner)?));
    Ok(())
}

/// A chord is two or more holds released in reverse: `{KC_LCTL,KC_C}`.
fn chord_at(actions: &[Action]) -> Option<Vec<u8>> {
    let downs: Vec<u8> = actions
        .iter()
        .map_while(|action| match action {
            Action::Down(key) => Some(*key),
            _ => None,
        })
        .collect();
    if downs.len() < 2 {
        return None;
    }
    let ups = &actions[downs.len()..];
    if ups.len() < downs.len() {
        return None;
    }
    let reversed = downs
        .iter()
        .rev()
        .zip(ups)
        .all(|(key, action)| *action == Action::Up(*key));
    reversed.then_some(downs)
}

/// A macro presses basic keycodes only: the stored action has one byte.
fn basic_key(name: &str) -> Result<u8> {
    let name = name.trim();
    let code =
        keycode::parse(name).ok_or_else(|| Error::message(format!("{name} is not a keycode")))?;
    if code == 0 || code > 0xFF {
        return Err(Error::message(format!(
            "{name} is not a basic key; a macro can only press keys such as KC_A or KC_LCTL"
        )));
    }
    Ok(code as u8)
}

fn key_name(key: u8) -> String {
    keycode::long_name(u16::from(key), &[])
}

fn typable(ch: char) -> Result<char> {
    if (' '..='~').contains(&ch) {
        Ok(ch)
    } else {
        Err(Error::message(format!(
            "{ch:?} cannot be typed by a macro; use plain ASCII or a {{KC_…}} key"
        )))
    }
}

fn parse_hex(hex: &str) -> Result<Vec<u8>> {
    let digits: String = hex.chars().filter(|ch| !ch.is_whitespace()).collect();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return Err(Error::message("raw bytes are pairs of hex digits"));
    }
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.as_bytes().chunks(2) {
        let pair = std::str::from_utf8(pair).unwrap_or_default();
        let byte = u8::from_str_radix(pair, 16)
            .map_err(|_| Error::message(format!("{pair} is not a hex byte")))?;
        if byte == 0 {
            return Err(Error::message("a raw 00 would end the macro"));
        }
        bytes.push(byte);
    }
    Ok(bytes)
}

fn push_text(actions: &mut Vec<Action>, ch: char) {
    if let Some(Action::Text(text)) = actions.last_mut() {
        text.push(ch);
    } else {
        actions.push(Action::Text(ch.to_string()));
    }
}

fn push_raw(actions: &mut Vec<Action>, bytes: &[u8]) {
    if let Some(Action::Raw(raw)) = actions.last_mut() {
        raw.extend_from_slice(bytes);
    } else {
        actions.push(Action::Raw(bytes.to_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_actions_decode_the_qmk_way() {
        let bytes = b"Hi\x01\x01\x28\x01\x02\xE1a\x01\x03\xE1\x01\x04250|";
        assert_eq!(
            parse(bytes),
            vec![
                Action::Text("Hi".into()),
                Action::Tap(0x28),
                Action::Down(0xE1),
                Action::Text("a".into()),
                Action::Up(0xE1),
                Action::Delay(250),
            ]
        );
        assert_eq!(encode(&parse(bytes)), bytes);
    }

    #[test]
    fn unknown_actions_are_kept_byte_for_byte() {
        // Vial's extended tap, then an action nobody defines.
        let bytes = b"x\x01\x05\x10\x77y\x01\x09zz";
        let actions = parse(bytes);
        assert_eq!(
            actions,
            vec![
                Action::Text("x".into()),
                Action::Raw(b"\x01\x05\x10\x77".to_vec()),
                Action::Text("y".into()),
                Action::Raw(b"\x01\x09zz".to_vec()),
            ]
        );
        assert_eq!(encode(&actions), bytes);
        assert_eq!(from_text(&to_text(&actions)).unwrap(), actions);
    }

    #[test]
    fn a_buffer_splits_on_nul_and_pads_missing_macros() {
        let macros = parse_buffer(b"ab\0\x01\x01\x04\0", 3);
        assert_eq!(macros[0], vec![Action::Text("ab".into())]);
        assert_eq!(macros[1], vec![Action::Tap(0x04)]);
        assert_eq!(macros[2], vec![]);
        let bytes = encode_buffer(&macros, 12).unwrap();
        assert_eq!(bytes, b"ab\0\x01\x01\x04\0\0\0\0\0\0");
        assert!(encode_buffer(&macros, 6).is_err());
    }

    #[test]
    fn text_syntax_round_trips() {
        let text = "Hi{KC_ENTER}{+KC_LEFT_SHIFT}a{-KC_LEFT_SHIFT}{250}{KC_LEFT_CTRL,KC_C}\\{\\\\";
        let actions = from_text(text).unwrap();
        assert_eq!(
            actions,
            vec![
                Action::Text("Hi".into()),
                Action::Tap(0x28),
                Action::Down(0xE1),
                Action::Text("a".into()),
                Action::Up(0xE1),
                Action::Delay(250),
                Action::Down(0xE0),
                Action::Down(0x06),
                Action::Up(0x06),
                Action::Up(0xE0),
                Action::Text("{\\".into()),
            ]
        );
        assert_eq!(to_text(&actions), text);
    }

    #[test]
    fn short_names_and_aliases_are_accepted() {
        assert_eq!(from_text("{KC_ENT}").unwrap(), vec![Action::Tap(0x28)]);
        assert_eq!(from_text("{ kc_a }").unwrap(), vec![Action::Tap(0x04)]);
    }

    #[test]
    fn text_that_cannot_be_stored_is_refused() {
        assert!(from_text("{KC_A").is_err());
        assert!(from_text("{}").is_err());
        assert!(from_text("{NOPE}").is_err());
        assert!(from_text("{MO(1)}").is_err());
        assert!(from_text("{100000}").is_err());
        assert!(from_text("café").is_err());
        assert!(from_text("{raw:0100}").is_err());
        assert!(from_text("trailing\\").is_err());
    }

    #[test]
    fn reading_and_writing_go_through_the_buffer_frames() {
        let mut stored = b"one\0two\0".to_vec();
        stored.resize(40, 0);
        let mut writes: Vec<(usize, Vec<u8>)> = Vec::new();
        let mut transact = |request: Report| -> Result<Report> {
            let offset = usize::from(u16::from_be_bytes([request[1], request[2]]));
            let size = usize::from(request[3]);
            let mut reply = request;
            match request[0] {
                protocol::MACRO_GET_BUFFER => {
                    reply[4..4 + size].copy_from_slice(&stored[offset..offset + size]);
                }
                protocol::MACRO_SET_BUFFER => {
                    stored[offset..offset + size].copy_from_slice(&request[4..4 + size]);
                    writes.push((offset, request[4..4 + size].to_vec()));
                }
                _ => reply[0] = 0xFF,
            }
            Ok(reply)
        };
        let mut macros = Macros::read_from_device(&mut transact, 3, 40).unwrap();
        assert_eq!(macros.macros[1], vec![Action::Text("two".into())]);
        macros
            .set(2, from_text("{KC_A}").unwrap(), &mut transact)
            .unwrap();
        assert!(macros.set(3, Vec::new(), &mut transact).is_err());
        let reread = Macros::read_from_device(&mut transact, 3, 40).unwrap();
        assert_eq!(reread, macros);
        assert_eq!(reread.used(), 4 + 4 + 4);
        // The guard byte is set first and cleared last.
        assert_eq!(writes.first(), Some(&(39, vec![0xFF])));
        assert_eq!(writes.last(), Some(&(39, vec![0x00])));
    }

    #[test]
    fn a_macro_too_big_for_the_buffer_changes_nothing() {
        let mut macros = Macros {
            size: 4,
            macros: vec![Vec::new()],
        };
        let mut transact = |_: Report| -> Result<Report> { panic!("nothing should be written") };
        assert!(
            macros
                .set(0, vec![Action::Text("hello".into())], &mut transact)
                .is_err()
        );
        assert_eq!(macros.macros[0], vec![]);
    }
}
