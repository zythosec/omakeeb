//! A VIA keyboard definition: the matrix, the KLE drawing, and the layout
//! options that choose between keys sharing one spot (ANSI and ISO enter,
//! split and joined backspace).
//!
//! KLE positions follow the serialized format: a property object applies to
//! the next key, width and height reset after that key, and a new row moves
//! down one unit and back to the left. Rotation, when a definition uses it,
//! is drawn around the rotation origin.

use crate::error::{Error, Result};
use crate::lighting::Channel;

/// One switch, drawn in key units (1 is a 1u key).
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Second rectangle for a stepped or ISO enter, in the same coordinates.
    pub x2: f32,
    pub y2: f32,
    pub w2: f32,
    pub h2: f32,
    /// Degrees, clockwise, around `(rx, ry)`.
    pub rotation: f32,
    pub rx: f32,
    pub ry: f32,
    pub row: Option<u8>,
    pub col: Option<u8>,
    /// `(group, choice)` from the bottom-right legend. Absent means the key
    /// is drawn for every layout option.
    pub option: Option<(u8, u8)>,
    pub decal: bool,
    /// Text that is not a matrix address, for a decal.
    pub legend: String,
}

impl Key {
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn selectable(&self) -> bool {
        !self.decal && self.row.is_some() && self.col.is_some()
    }
}

/// A control in the definition's `labels` array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutGroup {
    pub label: String,
    /// Choice 0 is the default. A toggle is `Off` then `On`.
    pub choices: Vec<String>,
}

impl LayoutGroup {
    fn choice_count(&self) -> u32 {
        u32::try_from(self.choices.len()).unwrap_or(u32::MAX)
    }
}

/// A keyboard-specific keycode, `QK_KB + index`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomKey {
    pub name: String,
    pub title: String,
    pub short: String,
    pub index: u8,
}

/// Everything the drawing and the protocol need from one JSON definition.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub rows: u8,
    pub cols: u8,
    pub keys: Vec<Key>,
    pub groups: Vec<LayoutGroup>,
    pub lighting: Vec<Channel>,
    pub custom: Vec<CustomKey>,
}

impl Definition {
    pub fn parse(text: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|err| Error::message(format!("definition JSON: {err}")))?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &serde_json::Value) -> Result<Self> {
        let name = definition_name(value.get("name"))
            .ok_or_else(|| Error::message("definition needs a name"))?;
        let vendor_id = parse_id(value.get("vendorId"))
            .ok_or_else(|| Error::message("definition needs a vendorId"))?;
        let product_id = parse_id(value.get("productId"))
            .ok_or_else(|| Error::message("definition needs a productId"))?;
        let matrix = value
            .get("matrix")
            .ok_or_else(|| Error::message("definition needs a matrix"))?;
        let rows = matrix
            .get("rows")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| Error::message("matrix.rows must be a positive number"))?;
        let cols = matrix
            .get("cols")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| Error::message("matrix.cols must be a positive number"))?;
        let layouts = value
            .get("layouts")
            .ok_or_else(|| Error::message("definition needs layouts"))?;
        let keymap = layouts
            .get("keymap")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| Error::message("layouts.keymap must be a KLE array"))?;
        let groups = parse_groups(layouts.get("labels"));
        let keys = parse_kle(keymap)?;
        for key in &keys {
            if let Some(row) = key.row
                && row >= rows
            {
                return Err(Error::message(format!(
                    "key row {row} is outside the {rows}-row matrix"
                )));
            }
            if let Some(col) = key.col
                && col >= cols
            {
                return Err(Error::message(format!(
                    "key column {col} is outside the {cols}-column matrix"
                )));
            }
            if let Some((group, _)) = key.option
                && usize::from(group) >= groups.len()
            {
                return Err(Error::message(format!(
                    "key refers to layout group {group}, and the definition has {}",
                    groups.len()
                )));
            }
        }
        Ok(Self {
            name,
            vendor_id,
            product_id,
            rows,
            cols,
            keys,
            groups,
            lighting: parse_lighting(value.get("menus")),
            custom: parse_custom(value.get("customKeycodes")),
        })
    }

    /// A grid used when a keyboard answered but no definition names its shape.
    /// Every matrix position is a 1u key, so remapping still works.
    pub fn matrix_grid(
        name: impl Into<String>,
        vendor_id: u16,
        product_id: u16,
        rows: u8,
        cols: u8,
    ) -> Self {
        let mut keys = Vec::new();
        for row in 0..rows {
            for col in 0..cols {
                keys.push(Key {
                    x: f32::from(col),
                    y: f32::from(row),
                    w: 1.0,
                    h: 1.0,
                    x2: 0.0,
                    y2: 0.0,
                    w2: 0.0,
                    h2: 0.0,
                    rotation: 0.0,
                    rx: 0.0,
                    ry: 0.0,
                    row: Some(row),
                    col: Some(col),
                    option: None,
                    decal: false,
                    legend: String::new(),
                });
            }
        }
        Self {
            name: name.into(),
            vendor_id,
            product_id,
            rows,
            cols,
            keys,
            groups: Vec::new(),
            lighting: Vec::new(),
            custom: Vec::new(),
        }
    }

    pub fn bounds(&self) -> (f32, f32) {
        let mut width: f32 = 1.0;
        let mut height: f32 = 1.0;
        for key in &self.keys {
            width = width.max(key.x + key.w);
            height = height.max(key.y + key.h);
            if key.w2 > 0.0 {
                width = width.max(key.x + key.x2 + key.w2);
                height = height.max(key.y + key.y2 + key.h2);
            }
        }
        (width, height)
    }

    /// Keys drawn for the layout-option bitfield currently stored on the keyboard.
    pub fn visible_keys(&self, options: u32) -> Vec<&Key> {
        let choices = unpack_options(options, &self.groups);
        self.keys
            .iter()
            .filter(|key| key_matches(key, &choices))
            .collect()
    }

    pub fn choice(&self, options: u32, group: usize) -> u32 {
        unpack_options(options, &self.groups)
            .get(group)
            .copied()
            .unwrap_or(0)
    }

    /// Replace one group's choice and pack the bitfield the firmware stores.
    pub fn with_choice(&self, options: u32, group: usize, choice: u32) -> Result<u32> {
        if group >= self.groups.len() {
            return Err(Error::message("that layout group does not exist"));
        }
        let count = self.groups[group].choice_count();
        if choice >= count {
            return Err(Error::message("that layout choice does not exist"));
        }
        let mut choices = unpack_options(options, &self.groups);
        choices[group] = choice;
        Ok(pack_options(&choices, &self.groups))
    }
}

fn key_matches(key: &Key, choices: &[u32]) -> bool {
    match key.option {
        None => true,
        Some((group, choice)) => {
            choices.get(usize::from(group)).copied().unwrap_or(0) == u32::from(choice)
        }
    }
}

/// Bits required to store `n` choices, packed from the least significant bit.
fn bits_for(n: u32) -> u32 {
    if n <= 1 {
        0
    } else {
        32 - (n - 1).leading_zeros()
    }
}

pub fn unpack_options(value: u32, groups: &[LayoutGroup]) -> Vec<u32> {
    let mut shift = 0_u32;
    let mut choices = Vec::with_capacity(groups.len());
    for group in groups {
        let bits = bits_for(group.choice_count());
        let mask = if bits >= 32 {
            u32::MAX
        } else {
            (1_u32 << bits) - 1
        };
        let mut choice = (value >> shift) & mask;
        let count = group.choice_count();
        if count > 0 && choice >= count {
            choice = 0;
        }
        choices.push(choice);
        shift = shift.saturating_add(bits);
    }
    choices
}

pub fn pack_options(choices: &[u32], groups: &[LayoutGroup]) -> u32 {
    let mut value = 0_u32;
    let mut shift = 0_u32;
    for (index, group) in groups.iter().enumerate() {
        let bits = bits_for(group.choice_count());
        let choice = choices.get(index).copied().unwrap_or(0);
        if bits > 0 && shift < 32 {
            value |= choice << shift;
        }
        shift = shift.saturating_add(bits);
    }
    value
}

fn definition_name(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    value
        .get("options")
        .and_then(serde_json::Value::as_array)
        .and_then(|options| options.first())
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn parse_id(value: Option<&serde_json::Value>) -> Option<u16> {
    let value = value?;
    if let Some(number) = value.as_u64() {
        return u16::try_from(number).ok();
    }
    let text = value.as_str()?.trim();
    let (radix, digits) =
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (16, hex)
        } else {
            (10, text)
        };
    u16::from_str_radix(digits, radix).ok()
}

fn parse_groups(value: Option<&serde_json::Value>) -> Vec<LayoutGroup> {
    let Some(items) = value.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(label) = item.as_str() {
                return Some(LayoutGroup {
                    label: label.to_owned(),
                    choices: vec!["Off".to_owned(), "On".to_owned()],
                });
            }
            let list = item.as_array()?;
            let label = list.first()?.as_str()?.to_owned();
            let choices: Vec<String> = list
                .iter()
                .skip(1)
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect();
            if choices.is_empty() {
                return None;
            }
            Some(LayoutGroup { label, choices })
        })
        .collect()
}

fn parse_lighting(value: Option<&serde_json::Value>) -> Vec<Channel> {
    let mut found = Vec::new();
    collect_lighting(value, &mut found);
    // A combined backlight+underglow token contributes backlight above; look
    // through the serialized menus again for the underglow half.
    if let Some(text) = value.map(serde_json::Value::to_string)
        && text.contains("qmk_backlight_rgblight")
        && !found.contains(&Channel::RgbLight)
    {
        found.push(Channel::RgbLight);
    }
    found
}

fn collect_lighting(value: Option<&serde_json::Value>, found: &mut Vec<Channel>) {
    let Some(value) = value else {
        return;
    };
    match value {
        serde_json::Value::String(token) => {
            if let Some(channel) = Channel::from_menu_token(token)
                && !found.contains(&channel)
            {
                found.push(channel);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_lighting(Some(item), found);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if let Some(channel) = Channel::from_menu_token(key)
                    && !found.contains(&channel)
                {
                    found.push(channel);
                }
                if let Some(text) = child.as_str()
                    && let Some(channel) = Channel::from_menu_token(text)
                    && !found.contains(&channel)
                {
                    found.push(channel);
                }
                collect_lighting(Some(child), found);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn parse_custom(value: Option<&serde_json::Value>) -> Vec<CustomKey> {
    let Some(items) = value.and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let index = u8::try_from(index).ok()?;
            let name = item.get("name")?.as_str()?.to_owned();
            let title = item
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&name)
                .to_owned();
            let short = item
                .get("shortName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&name)
                .to_owned();
            Some(CustomKey {
                name,
                title,
                short,
                index,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
struct Cursor {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    x2: f32,
    y2: f32,
    w2: f32,
    h2: f32,
    r: f32,
    rx: f32,
    ry: f32,
    decal: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            x2: 0.0,
            y2: 0.0,
            w2: 0.0,
            h2: 0.0,
            r: 0.0,
            rx: 0.0,
            ry: 0.0,
            decal: false,
        }
    }
}

fn parse_kle(rows: &[serde_json::Value]) -> Result<Vec<Key>> {
    let mut cursor = Cursor::default();
    let mut keys = Vec::new();
    let mut first_row = true;
    for row in rows {
        let Some(items) = row.as_array() else {
            // A leading metadata object is part of a raw KLE file, not a key.
            continue;
        };
        if !first_row {
            cursor.y += 1.0;
            cursor.x = 0.0;
        }
        first_row = false;
        for item in items {
            match item {
                serde_json::Value::Object(props) => apply_props(&mut cursor, props),
                other => {
                    let label = kle_label(other);
                    keys.push(key_from_cursor(&cursor, &label));
                    cursor.x += cursor.w;
                    cursor.w = 1.0;
                    cursor.h = 1.0;
                    cursor.x2 = 0.0;
                    cursor.y2 = 0.0;
                    cursor.w2 = 0.0;
                    cursor.h2 = 0.0;
                    cursor.decal = false;
                }
            }
        }
    }
    if keys.is_empty() {
        return Err(Error::message("the layout has no keys"));
    }
    Ok(keys)
}

fn apply_props(cursor: &mut Cursor, props: &serde_json::Map<String, serde_json::Value>) {
    if let Some(value) = number(props.get("x")) {
        cursor.x += value;
    }
    if let Some(value) = number(props.get("y")) {
        cursor.y += value;
    }
    if let Some(value) = number(props.get("w")) {
        cursor.w = value;
    }
    if let Some(value) = number(props.get("h")) {
        cursor.h = value;
    }
    if let Some(value) = number(props.get("x2")) {
        cursor.x2 = value;
    }
    if let Some(value) = number(props.get("y2")) {
        cursor.y2 = value;
    }
    if let Some(value) = number(props.get("w2")) {
        cursor.w2 = value;
    }
    if let Some(value) = number(props.get("h2")) {
        cursor.h2 = value;
    }
    if props.contains_key("r") {
        if let Some(value) = number(props.get("r")) {
            cursor.r = value;
        }
        if !props.contains_key("rx") {
            cursor.rx = cursor.x;
        }
        if !props.contains_key("ry") {
            cursor.ry = cursor.y;
        }
    }
    if let Some(value) = number(props.get("rx")) {
        cursor.rx = value;
    }
    if let Some(value) = number(props.get("ry")) {
        cursor.ry = value;
    }
    if let Some(value) = props.get("d").and_then(serde_json::Value::as_bool) {
        cursor.decal = value;
    }
}

fn number(value: Option<&serde_json::Value>) -> Option<f32> {
    value.and_then(serde_json::Value::as_f64).map(|n| n as f32)
}

fn kle_label(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

fn key_from_cursor(cursor: &Cursor, label: &str) -> Key {
    let parts: Vec<&str> = label.split('\n').collect();
    let (row, col) = parts
        .first()
        .and_then(|text| parse_pair(text))
        .map_or((None, None), |(row, col)| (Some(row), Some(col)));
    let option = parts.get(3).and_then(|text| parse_pair(text));
    let legend = if row.is_none() {
        parts.first().copied().unwrap_or("").to_owned()
    } else {
        String::new()
    };
    Key {
        x: cursor.x,
        y: cursor.y,
        w: cursor.w,
        h: cursor.h,
        x2: cursor.x2,
        y2: cursor.y2,
        w2: cursor.w2,
        h2: cursor.h2,
        rotation: cursor.r,
        rx: cursor.rx,
        ry: cursor.ry,
        row,
        col,
        option,
        decal: cursor.decal || row.is_none(),
        legend,
    }
}

fn parse_pair(text: &str) -> Option<(u8, u8)> {
    let (left, right) = text.split_once(',')?;
    let left = left.trim().parse().ok()?;
    let right = right.trim().parse().ok()?;
    Some((left, right))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSI_ISO: &str = r#"{
      "name": "Example",
      "vendorId": "0xFEED",
      "productId": "0x0001",
      "matrix": {"rows": 2, "cols": 3},
      "layouts": {
        "labels": ["Split Backspace", ["Bottom", "ANSI", "ISO", "HHKB"]],
        "keymap": [
          [{ "w": 2 }, "0,0\n\n\n0,0", { "x": -2 }, "0,0\n\n\n0,1", "0,1\n\n\n0,1"],
          ["1,0", { "x": 1, "w": 1.5 }, "1,2"]
        ]
      },
      "menus": ["qmk_rgb_matrix"],
      "customKeycodes": [{ "name": "Mode", "title": "Cycle mode", "shortName": "Md" }]
    }"#;

    #[test]
    fn widths_reset_and_rows_step() {
        let definition = Definition::parse(ANSI_ISO).unwrap();
        assert_eq!(definition.vendor_id, 0xFEED);
        assert_eq!(definition.rows, 2);
        assert_eq!(definition.lighting, vec![Channel::RgbMatrix]);
        assert_eq!(definition.custom[0].short, "Md");
        let keys = &definition.keys;
        assert_eq!((keys[0].x, keys[0].w), (0.0, 2.0));
        assert_eq!((keys[1].x, keys[1].w), (0.0, 1.0));
        assert_eq!((keys[2].x, keys[2].w), (1.0, 1.0));
        assert_eq!((keys[3].x, keys[3].y, keys[3].w), (0.0, 1.0, 1.0));
        assert_eq!((keys[4].x, keys[4].w), (2.0, 1.5));
    }

    #[test]
    fn layout_options_show_one_backspace() {
        let definition = Definition::parse(ANSI_ISO).unwrap();
        assert_eq!(definition.groups.len(), 2);
        assert_eq!(definition.groups[1].choices, ["ANSI", "ISO", "HHKB"]);
        let joined = definition.visible_keys(0);
        assert!(joined.iter().any(|key| key.w == 2.0));
        assert!(!joined.iter().any(|key| key.option == Some((0, 1))));
        let split = definition.with_choice(0, 0, 1).unwrap();
        assert_eq!(split, 1);
        let split_keys = definition.visible_keys(split);
        assert!(!split_keys.iter().any(|key| key.w == 2.0));
        assert_eq!(
            split_keys
                .iter()
                .filter(|key| key.option == Some((0, 1)))
                .count(),
            2
        );
        // The bottom-row group occupies the next two bits.
        let iso = definition.with_choice(split, 1, 1).unwrap();
        assert_eq!(iso, 0b011);
        assert_eq!(definition.choice(iso, 1), 1);
    }

    #[test]
    fn a_key_outside_the_matrix_is_rejected() {
        let text = r#"{
          "name": "Bad",
          "vendorId": "0x1",
          "productId": "0x2",
          "matrix": {"rows": 1, "cols": 1},
          "layouts": {"keymap": [["0,1"]]}
        }"#;
        assert!(Definition::parse(text).is_err());
    }

    #[test]
    fn a_decal_is_drawn_and_not_a_switch() {
        let text = r#"{
          "name": "Decal",
          "vendorId": 1,
          "productId": 2,
          "matrix": {"rows": 1, "cols": 1},
          "layouts": {"keymap": [[{"d": true}, "Logo", "0,0"]]}
        }"#;
        let definition = Definition::parse(text).unwrap();
        assert!(definition.keys[0].decal);
        assert_eq!(definition.keys[0].legend, "Logo");
        assert!(!definition.keys[0].selectable());
        assert!(definition.keys[1].selectable());
    }
}
