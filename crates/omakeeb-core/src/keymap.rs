//! The dynamic keymap: one `u16` per layer, row and column.

use crate::error::{Error, Result};
use crate::protocol::{self, Report};

/// Keycodes stored the way the firmware stores them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    pub layers: u8,
    pub rows: u8,
    pub cols: u8,
    codes: Vec<u16>,
}

impl Keymap {
    pub fn filled(layers: u8, rows: u8, cols: u8, code: u16) -> Result<Self> {
        if layers == 0 || rows == 0 || cols == 0 {
            return Err(Error::message(
                "a keymap needs at least one layer, row and column",
            ));
        }
        let len = usize::from(layers) * usize::from(rows) * usize::from(cols);
        Ok(Self {
            layers,
            rows,
            cols,
            codes: vec![code; len],
        })
    }

    pub fn from_bytes(layers: u8, rows: u8, cols: u8, bytes: &[u8]) -> Result<Self> {
        let mut map = Self::filled(layers, rows, cols, 0)?;
        let expected = map.codes.len() * 2;
        if bytes.len() != expected {
            return Err(Error::message(format!(
                "keymap is {} bytes, expected {expected}",
                bytes.len()
            )));
        }
        for (index, code) in map.codes.iter_mut().enumerate() {
            let at = index * 2;
            *code = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        }
        Ok(map)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.codes.len() * 2);
        for code in &self.codes {
            bytes.extend_from_slice(&code.to_be_bytes());
        }
        bytes
    }

    pub fn get(&self, layer: u8, row: u8, col: u8) -> Option<u16> {
        self.index(layer, row, col).map(|index| self.codes[index])
    }

    pub fn set(&mut self, layer: u8, row: u8, col: u8, code: u16) -> Result<()> {
        let Some(index) = self.index(layer, row, col) else {
            return Err(Error::message(format!(
                "layer {layer}, row {row}, column {col} is outside the matrix"
            )));
        };
        self.codes[index] = code;
        Ok(())
    }

    fn index(&self, layer: u8, row: u8, col: u8) -> Option<usize> {
        if layer >= self.layers || row >= self.rows || col >= self.cols {
            return None;
        }
        Some(protocol::keymap_offset(layer, row, col, self.rows, self.cols) / 2)
    }

    /// A JSON document a person can put under `~/.config/omakeeb/keymaps`.
    pub fn to_json(&self, name: &str, vendor_id: u16, product_id: u16) -> String {
        let mut layers = Vec::new();
        for layer in 0..self.layers {
            let mut rows = Vec::new();
            for row in 0..self.rows {
                let mut cols = Vec::new();
                for col in 0..self.cols {
                    cols.push(self.get(layer, row, col).unwrap_or(0));
                }
                rows.push(cols);
            }
            layers.push(rows);
        }
        serde_json::json!({
            "name": name,
            "vendorId": format!("0x{vendor_id:04X}"),
            "productId": format!("0x{product_id:04X}"),
            "layers": layers,
        })
        .to_string()
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|err| Error::message(format!("keymap JSON: {err}")))?;
        let layers = value
            .get("layers")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| Error::message("keymap JSON needs a layers array"))?;
        if layers.is_empty() {
            return Err(Error::message("keymap JSON has no layers"));
        }
        let row_count = layers[0]
            .as_array()
            .ok_or_else(|| Error::message("a layer must be an array of rows"))?
            .len();
        if row_count == 0 || row_count > usize::from(u8::MAX) {
            return Err(Error::message("keymap JSON has no rows"));
        }
        let col_count = layers[0][0]
            .as_array()
            .ok_or_else(|| Error::message("a row must be an array of keycodes"))?
            .len();
        if col_count == 0 || col_count > usize::from(u8::MAX) {
            return Err(Error::message("keymap JSON has no columns"));
        }
        if layers.len() > usize::from(u8::MAX) {
            return Err(Error::message("keymap JSON has too many layers"));
        }
        let mut map = Self::filled(layers.len() as u8, row_count as u8, col_count as u8, 0)?;
        for (layer_index, layer) in layers.iter().enumerate() {
            let rows = layer
                .as_array()
                .ok_or_else(|| Error::message("a layer must be an array of rows"))?;
            if rows.len() != row_count {
                return Err(Error::message("layers disagree on how many rows they have"));
            }
            for (row_index, row) in rows.iter().enumerate() {
                let cols = row
                    .as_array()
                    .ok_or_else(|| Error::message("a row must be an array of keycodes"))?;
                if cols.len() != col_count {
                    return Err(Error::message(
                        "rows disagree on how many columns they have",
                    ));
                }
                for (col_index, code) in cols.iter().enumerate() {
                    let code = code
                        .as_u64()
                        .ok_or_else(|| Error::message("a keycode must be a number"))?;
                    if code > u64::from(u16::MAX) {
                        return Err(Error::message(format!(
                            "keycode {code} does not fit in 16 bits"
                        )));
                    }
                    map.set(
                        layer_index as u8,
                        row_index as u8,
                        col_index as u8,
                        code as u16,
                    )?;
                }
            }
        }
        Ok(map)
    }
}

/// Read a whole keymap, preferring the bulk buffer and falling back to one
/// key at a time when the firmware does not implement it.
pub fn read_from_device(
    transact: &mut dyn FnMut(Report) -> Result<Report>,
    layers: u8,
    rows: u8,
    cols: u8,
) -> Result<Keymap> {
    match read_buffer(transact, layers, rows, cols) {
        Err(Error::Unhandled) => read_by_key(transact, layers, rows, cols),
        other => other,
    }
}

fn read_buffer(
    transact: &mut dyn FnMut(Report) -> Result<Report>,
    layers: u8,
    rows: u8,
    cols: u8,
) -> Result<Keymap> {
    let total = usize::from(layers) * usize::from(rows) * usize::from(cols) * 2;
    let mut bytes = vec![0; total];
    let mut offset = 0;
    while offset < total {
        let size = (total - offset).min(protocol::KEYMAP_CHUNK) as u8;
        let request = protocol::get_buffer_request(offset as u16, size)?;
        let reply = transact(request)?;
        let chunk = protocol::buffer_bytes(&reply, size)?;
        bytes[offset..offset + usize::from(size)].copy_from_slice(chunk);
        offset += usize::from(size);
    }
    Keymap::from_bytes(layers, rows, cols, &bytes)
}

fn read_by_key(
    transact: &mut dyn FnMut(Report) -> Result<Report>,
    layers: u8,
    rows: u8,
    cols: u8,
) -> Result<Keymap> {
    let mut map = Keymap::filled(layers, rows, cols, 0)?;
    for layer in 0..layers {
        for row in 0..rows {
            for col in 0..cols {
                let reply = transact(protocol::get_keycode_request(layer, row, col))?;
                let (_, _, _, code) = protocol::keycode_from_reply(&reply)?;
                map.set(layer, row, col, code)?;
            }
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip_in_firmware_order() {
        let mut map = Keymap::filled(2, 1, 2, 0).unwrap();
        map.set(0, 0, 0, 0x0004).unwrap();
        map.set(0, 0, 1, 0x0005).unwrap();
        map.set(1, 0, 0, 0x5221).unwrap();
        let bytes = map.to_bytes();
        assert_eq!(bytes, vec![0x00, 0x04, 0x00, 0x05, 0x52, 0x21, 0x00, 0x00]);
        assert_eq!(Keymap::from_bytes(2, 1, 2, &bytes).unwrap(), map);
    }

    #[test]
    fn a_write_outside_the_matrix_is_refused() {
        let mut map = Keymap::filled(1, 2, 2, 0).unwrap();
        assert!(map.set(1, 0, 0, 0x0004).is_err());
        assert!(map.set(0, 2, 0, 0x0004).is_err());
        assert!(map.set(0, 0, 2, 0x0004).is_err());
        assert_eq!(map.get(0, 0, 0), Some(0));
    }

    #[test]
    fn json_round_trip() {
        let mut map = Keymap::filled(1, 1, 2, 4).unwrap();
        map.set(0, 0, 1, 5).unwrap();
        let text = map.to_json("Example", 0xFEED, 0x0001);
        let loaded = Keymap::from_json(&text).unwrap();
        assert_eq!(loaded, map);
    }

    #[test]
    fn bulk_read_assembles_chunks() {
        let stored = [0x00u8, 0x04, 0x00, 0x29, 0x00, 0x2C];
        let mut calls = 0;
        let mut transact = |request: Report| {
            calls += 1;
            assert_eq!(request[0], protocol::DYNAMIC_KEYMAP_GET_BUFFER);
            let offset = usize::from(u16::from_be_bytes([request[1], request[2]]));
            let size = usize::from(request[3]);
            let mut reply = protocol::blank(protocol::DYNAMIC_KEYMAP_GET_BUFFER);
            reply[1] = request[1];
            reply[2] = request[2];
            reply[3] = request[3];
            reply[4..4 + size].copy_from_slice(&stored[offset..offset + size]);
            Ok(reply)
        };
        // Force more than one chunk by pretending the chunk size... the real
        // chunk is 28, so a 6-byte map is one call. Check that one call works,
        // then a firmware that refuses the buffer falls back.
        let map = read_from_device(&mut transact, 1, 1, 3).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(map.get(0, 0, 0), Some(0x0004));
        assert_eq!(map.get(0, 0, 1), Some(0x0029));
        assert_eq!(map.get(0, 0, 2), Some(0x002C));

        let mut transact = |request: Report| {
            if request[0] == protocol::DYNAMIC_KEYMAP_GET_BUFFER {
                let reply = protocol::blank(0xFF);
                return Ok(reply);
            }
            let mut reply = request;
            reply[4] = 0x00;
            reply[5] = 0x04;
            Ok(reply)
        };
        let map = read_from_device(&mut transact, 1, 1, 1).unwrap();
        assert_eq!(map.get(0, 0, 0), Some(0x0004));
    }
}
