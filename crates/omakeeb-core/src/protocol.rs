//! VIA report frames.
//!
//! A report is 32 bytes. Byte 0 is the command. The firmware answers in the
//! same buffer and sends it back; `0xFF` in byte 0 means the command is not
//! implemented. Multi-byte integers are big-endian.

use crate::error::{Error, Result};

pub const REPORT_LEN: usize = 32;
/// Bytes of keymap the firmware will copy into one `get_buffer` reply.
pub const KEYMAP_CHUNK: usize = 28;

pub const GET_PROTOCOL_VERSION: u8 = 0x01;
pub const GET_KEYBOARD_VALUE: u8 = 0x02;
pub const SET_KEYBOARD_VALUE: u8 = 0x03;
pub const DYNAMIC_KEYMAP_GET_KEYCODE: u8 = 0x04;
pub const DYNAMIC_KEYMAP_SET_KEYCODE: u8 = 0x05;
pub const DYNAMIC_KEYMAP_RESET: u8 = 0x06;
pub const CUSTOM_SET_VALUE: u8 = 0x07;
pub const CUSTOM_GET_VALUE: u8 = 0x08;
pub const CUSTOM_SAVE: u8 = 0x09;
pub const EEPROM_RESET: u8 = 0x0A;
pub const BOOTLOADER_JUMP: u8 = 0x0B;
pub const MACRO_GET_COUNT: u8 = 0x0C;
pub const MACRO_GET_BUFFER_SIZE: u8 = 0x0D;
pub const MACRO_GET_BUFFER: u8 = 0x0E;
pub const MACRO_SET_BUFFER: u8 = 0x0F;
pub const MACRO_RESET: u8 = 0x10;
pub const DYNAMIC_KEYMAP_GET_LAYER_COUNT: u8 = 0x11;
pub const DYNAMIC_KEYMAP_GET_BUFFER: u8 = 0x12;

pub const VALUE_LAYOUT_OPTIONS: u8 = 0x02;
pub const VALUE_FIRMWARE_VERSION: u8 = 0x04;
pub const VALUE_DEVICE_INDICATION: u8 = 0x05;
pub const VALUE_KEYCODES_VERSION: u8 = 0x06;

pub const CHANNEL_BACKLIGHT: u8 = 1;
pub const CHANNEL_RGBLIGHT: u8 = 2;
pub const CHANNEL_RGB_MATRIX: u8 = 3;
pub const CHANNEL_LED_MATRIX: u8 = 5;

pub const LIGHT_BRIGHTNESS: u8 = 1;
pub const LIGHT_EFFECT: u8 = 2;
pub const LIGHT_SPEED: u8 = 3;
pub const LIGHT_COLOR: u8 = 4;

pub type Report = [u8; REPORT_LEN];

pub fn blank(command: u8) -> Report {
    let mut report = [0; REPORT_LEN];
    report[0] = command;
    report
}

pub fn ensure_command(report: &Report, command: u8) -> Result<()> {
    if report[0] == 0xFF {
        return Err(Error::Unhandled);
    }
    if report[0] != command {
        return Err(Error::message(format!(
            "keyboard replied with command {:#04x}, expected {command:#04x}",
            report[0]
        )));
    }
    Ok(())
}

pub fn protocol_version(report: &Report) -> Result<u16> {
    ensure_command(report, GET_PROTOCOL_VERSION)?;
    Ok(u16::from_be_bytes([report[1], report[2]]))
}

pub fn layer_count_request() -> Report {
    blank(DYNAMIC_KEYMAP_GET_LAYER_COUNT)
}

pub fn layer_count(report: &Report) -> Result<u8> {
    ensure_command(report, DYNAMIC_KEYMAP_GET_LAYER_COUNT)?;
    let count = report[1];
    if count == 0 {
        return Err(Error::message("keyboard reported zero layers"));
    }
    Ok(count)
}

pub fn u32_value_request(command: u8, id: u8) -> Report {
    let mut report = blank(command);
    report[1] = id;
    report
}

pub fn u32_from_value(report: &Report, command: u8, id: u8) -> Result<u32> {
    ensure_command(report, command)?;
    if report[1] != id {
        return Err(Error::message("keyboard replied to a different value"));
    }
    Ok(u32::from_be_bytes([
        report[2], report[3], report[4], report[5],
    ]))
}

pub fn set_u32_value(id: u8, value: u32) -> Report {
    let mut report = blank(SET_KEYBOARD_VALUE);
    report[1] = id;
    let bytes = value.to_be_bytes();
    report[2..6].copy_from_slice(&bytes);
    report
}

pub fn get_keycode_request(layer: u8, row: u8, col: u8) -> Report {
    let mut report = blank(DYNAMIC_KEYMAP_GET_KEYCODE);
    report[1] = layer;
    report[2] = row;
    report[3] = col;
    report
}

pub fn keycode_from_reply(report: &Report) -> Result<(u8, u8, u8, u16)> {
    ensure_command(report, DYNAMIC_KEYMAP_GET_KEYCODE)?;
    Ok((
        report[1],
        report[2],
        report[3],
        u16::from_be_bytes([report[4], report[5]]),
    ))
}

pub fn set_keycode_request(layer: u8, row: u8, col: u8, code: u16) -> Report {
    let mut report = blank(DYNAMIC_KEYMAP_SET_KEYCODE);
    report[1] = layer;
    report[2] = row;
    report[3] = col;
    let [hi, lo] = code.to_be_bytes();
    report[4] = hi;
    report[5] = lo;
    report
}

pub fn get_buffer_request(offset: u16, size: u8) -> Result<Report> {
    if size as usize > KEYMAP_CHUNK {
        return Err(Error::message(format!(
            "keymap read of {size} bytes exceeds the {KEYMAP_CHUNK}-byte frame"
        )));
    }
    let mut report = blank(DYNAMIC_KEYMAP_GET_BUFFER);
    let [hi, lo] = offset.to_be_bytes();
    report[1] = hi;
    report[2] = lo;
    report[3] = size;
    Ok(report)
}

pub fn buffer_bytes(report: &Report, size: u8) -> Result<&[u8]> {
    chunk_bytes(report, DYNAMIC_KEYMAP_GET_BUFFER, size)
}

pub fn macro_count(report: &Report) -> Result<u8> {
    ensure_command(report, MACRO_GET_COUNT)?;
    Ok(report[1])
}

pub fn macro_buffer_size(report: &Report) -> Result<u16> {
    ensure_command(report, MACRO_GET_BUFFER_SIZE)?;
    Ok(u16::from_be_bytes([report[1], report[2]]))
}

/// The macro buffer is read and written in the same frame as the keymap
/// buffer: offset, size, then up to 28 bytes.
pub fn macro_get_request(offset: u16, size: u8) -> Result<Report> {
    let mut report = get_buffer_request(offset, size)?;
    report[0] = MACRO_GET_BUFFER;
    Ok(report)
}

pub fn macro_bytes(report: &Report, size: u8) -> Result<&[u8]> {
    chunk_bytes(report, MACRO_GET_BUFFER, size)
}

pub fn macro_set_request(offset: u16, data: &[u8]) -> Result<Report> {
    if data.len() > KEYMAP_CHUNK {
        return Err(Error::message(format!(
            "macro write of {} bytes exceeds the {KEYMAP_CHUNK}-byte frame",
            data.len()
        )));
    }
    let mut report = blank(MACRO_SET_BUFFER);
    let [hi, lo] = offset.to_be_bytes();
    report[1] = hi;
    report[2] = lo;
    report[3] = data.len() as u8;
    report[4..4 + data.len()].copy_from_slice(data);
    Ok(report)
}

fn chunk_bytes(report: &Report, command: u8, size: u8) -> Result<&[u8]> {
    ensure_command(report, command)?;
    let size = usize::from(size);
    let end = 4 + size;
    if end > report.len() {
        return Err(Error::message("buffer chunk does not fit in a report"));
    }
    Ok(&report[4..end])
}

pub fn lighting_get(channel: u8, value: u8) -> Report {
    let mut report = blank(CUSTOM_GET_VALUE);
    report[1] = channel;
    report[2] = value;
    report
}

pub fn lighting_set(channel: u8, value: u8, data: &[u8]) -> Result<Report> {
    if data.len() > 8 {
        return Err(Error::message("lighting value is too long for a report"));
    }
    let mut report = blank(CUSTOM_SET_VALUE);
    report[1] = channel;
    report[2] = value;
    report[3..3 + data.len()].copy_from_slice(data);
    Ok(report)
}

pub fn lighting_save(channel: u8) -> Report {
    let mut report = blank(CUSTOM_SAVE);
    report[1] = channel;
    report
}

pub fn lighting_byte(report: &Report, channel: u8, value: u8) -> Result<u8> {
    ensure_command(report, CUSTOM_GET_VALUE)?;
    if report[1] != channel || report[2] != value {
        return Err(Error::message(
            "keyboard replied to a different lighting value",
        ));
    }
    Ok(report[3])
}

pub fn lighting_color(report: &Report, channel: u8) -> Result<(u8, u8)> {
    ensure_command(report, CUSTOM_GET_VALUE)?;
    if report[1] != channel || report[2] != LIGHT_COLOR {
        return Err(Error::message(
            "keyboard replied to a different lighting value",
        ));
    }
    Ok((report[3], report[4]))
}

/// Byte offset of one key in the dynamic keymap: layer, then row, then column,
/// two bytes each, big-endian.
pub fn keymap_offset(layer: u8, row: u8, col: u8, rows: u8, cols: u8) -> usize {
    let rows = usize::from(rows);
    let cols = usize::from(cols);
    ((usize::from(layer) * rows + usize::from(row)) * cols + usize::from(col)) * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_big_endian() {
        let mut report = blank(GET_PROTOCOL_VERSION);
        report[1] = 0x00;
        report[2] = 0x0D;
        assert_eq!(protocol_version(&report).unwrap(), 13);
    }

    #[test]
    fn unhandled_is_distinct_from_a_mismatch() {
        let mut report = blank(0xFF);
        assert!(matches!(protocol_version(&report), Err(Error::Unhandled)));
        report[0] = GET_KEYBOARD_VALUE;
        assert!(matches!(protocol_version(&report), Err(Error::Message(_))));
    }

    #[test]
    fn set_keycode_round_trips_through_the_frame() {
        let request = set_keycode_request(2, 3, 4, 0x5221);
        assert_eq!(
            &request[..6],
            &[DYNAMIC_KEYMAP_SET_KEYCODE, 2, 3, 4, 0x52, 0x21]
        );
    }

    #[test]
    fn layout_options_are_four_big_endian_bytes() {
        let request = set_u32_value(VALUE_LAYOUT_OPTIONS, 0x00_00_01_02);
        assert_eq!(request[1], VALUE_LAYOUT_OPTIONS);
        assert_eq!(&request[2..6], &[0x00, 0x00, 0x01, 0x02]);
        assert_eq!(
            u32_from_value(&request, SET_KEYBOARD_VALUE, VALUE_LAYOUT_OPTIONS).unwrap(),
            0x102
        );
    }

    #[test]
    fn keymap_offset_matches_qmk_addressing() {
        // layer 1, row 1, col 2, on a 2×3 matrix: (1*2+1)*3+2 = 11 keys in, ×2.
        assert_eq!(keymap_offset(1, 1, 2, 2, 3), 22);
        assert_eq!(keymap_offset(0, 0, 0, 5, 14), 0);
    }

    #[test]
    fn buffer_read_rejects_an_oversized_chunk() {
        assert!(get_buffer_request(0, 29).is_err());
        let report = get_buffer_request(0x0102, 2).unwrap();
        assert_eq!(&report[..4], &[DYNAMIC_KEYMAP_GET_BUFFER, 0x01, 0x02, 2]);
    }

    #[test]
    fn macro_frames_carry_offset_size_and_data() {
        let read = macro_get_request(0x0120, 28).unwrap();
        assert_eq!(&read[..4], &[MACRO_GET_BUFFER, 0x01, 0x20, 28]);
        let write = macro_set_request(0x0003, b"hi").unwrap();
        assert_eq!(&write[..6], &[MACRO_SET_BUFFER, 0x00, 0x03, 2, b'h', b'i']);
        assert!(macro_set_request(0, &[0; 29]).is_err());
        let mut size = blank(MACRO_GET_BUFFER_SIZE);
        size[1] = 0x03;
        size[2] = 0xAC;
        assert_eq!(macro_buffer_size(&size).unwrap(), 940);
    }
}
