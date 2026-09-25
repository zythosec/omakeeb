//! Something that exchanges one 32-byte VIA report.
//!
//! The HID backend talks to a keyboard. [`MemoryKeyboard`] is the same
//! conversation with no hardware, used by the demo and by tests.

use crate::error::{Error, Result};
use crate::keymap::Keymap;
use crate::lighting::{Channel, Lighting};
use crate::protocol::{self, Report};

pub trait Link {
    fn transact(&mut self, report: Report) -> Result<Report>;
}

/// A keyboard that keeps its keymap in memory and answers the commands the
/// configurator sends.
#[derive(Clone, Debug)]
pub struct MemoryKeyboard {
    pub protocol: u16,
    pub firmware: u32,
    pub keymap: Keymap,
    pub layout_options: u32,
    pub lighting: Vec<Lighting>,
    pub macro_count: u8,
    /// The macro buffer as the firmware stores it, NULs and all.
    pub macro_buffer: Vec<u8>,
}

impl MemoryKeyboard {
    /// QMK's defaults: sixteen macros sharing what EEPROM is left, here 512
    /// bytes.
    pub fn new(keymap: Keymap) -> Self {
        Self {
            protocol: 13,
            firmware: 0,
            keymap,
            layout_options: 0,
            lighting: Vec::new(),
            macro_count: 16,
            macro_buffer: vec![0; 512],
        }
    }
}

impl Link for MemoryKeyboard {
    fn transact(&mut self, report: Report) -> Result<Report> {
        let mut reply = report;
        match report[0] {
            protocol::GET_PROTOCOL_VERSION => {
                let [hi, lo] = self.protocol.to_be_bytes();
                reply[1] = hi;
                reply[2] = lo;
            }
            protocol::GET_KEYBOARD_VALUE => match report[1] {
                protocol::VALUE_LAYOUT_OPTIONS => write_u32(&mut reply, self.layout_options),
                protocol::VALUE_FIRMWARE_VERSION | protocol::VALUE_KEYCODES_VERSION => {
                    write_u32(&mut reply, self.firmware);
                }
                _ => reply[0] = 0xFF,
            },
            protocol::SET_KEYBOARD_VALUE => {
                if report[1] == protocol::VALUE_LAYOUT_OPTIONS {
                    self.layout_options =
                        u32::from_be_bytes([report[2], report[3], report[4], report[5]]);
                } else if report[1] != protocol::VALUE_DEVICE_INDICATION {
                    reply[0] = 0xFF;
                }
            }
            protocol::DYNAMIC_KEYMAP_GET_LAYER_COUNT => reply[1] = self.keymap.layers,
            protocol::DYNAMIC_KEYMAP_GET_KEYCODE => {
                let code = self
                    .keymap
                    .get(report[1], report[2], report[3])
                    .ok_or_else(|| Error::message("keycode read is outside the matrix"))?;
                let [hi, lo] = code.to_be_bytes();
                reply[4] = hi;
                reply[5] = lo;
            }
            protocol::DYNAMIC_KEYMAP_SET_KEYCODE => {
                let code = u16::from_be_bytes([report[4], report[5]]);
                self.keymap.set(report[1], report[2], report[3], code)?;
            }
            protocol::DYNAMIC_KEYMAP_GET_BUFFER => {
                let offset = usize::from(u16::from_be_bytes([report[1], report[2]]));
                let size = usize::from(report[3]);
                let bytes = self.keymap.to_bytes();
                if offset.saturating_add(size) > bytes.len() || size > protocol::KEYMAP_CHUNK {
                    return Err(Error::message("keymap read is outside the buffer"));
                }
                reply[4..4 + size].copy_from_slice(&bytes[offset..offset + size]);
            }
            protocol::MACRO_GET_COUNT => reply[1] = self.macro_count,
            protocol::MACRO_GET_BUFFER_SIZE => {
                let [hi, lo] = (self.macro_buffer.len() as u16).to_be_bytes();
                reply[1] = hi;
                reply[2] = lo;
            }
            protocol::MACRO_GET_BUFFER | protocol::MACRO_SET_BUFFER => {
                let offset = usize::from(u16::from_be_bytes([report[1], report[2]]));
                let size = usize::from(report[3]);
                if offset.saturating_add(size) > self.macro_buffer.len()
                    || size > protocol::KEYMAP_CHUNK
                {
                    return Err(Error::message("macro access is outside the buffer"));
                }
                let stored = &mut self.macro_buffer[offset..offset + size];
                if report[0] == protocol::MACRO_GET_BUFFER {
                    reply[4..4 + size].copy_from_slice(stored);
                } else {
                    stored.copy_from_slice(&report[4..4 + size]);
                }
            }
            protocol::MACRO_RESET => self.macro_buffer.fill(0),
            protocol::DYNAMIC_KEYMAP_RESET => {
                self.keymap =
                    Keymap::filled(self.keymap.layers, self.keymap.rows, self.keymap.cols, 0)?;
            }
            protocol::CUSTOM_GET_VALUE => {
                let light = self
                    .lighting
                    .iter()
                    .find(|light| light.channel.id() == report[1])
                    .ok_or(Error::Unhandled)?;
                match report[2] {
                    protocol::LIGHT_BRIGHTNESS => reply[3] = light.brightness,
                    protocol::LIGHT_EFFECT => reply[3] = light.effect,
                    protocol::LIGHT_SPEED => reply[3] = light.speed,
                    protocol::LIGHT_COLOR => {
                        reply[3] = light.hue;
                        reply[4] = light.saturation;
                    }
                    _ => reply[0] = 0xFF,
                }
            }
            protocol::CUSTOM_SET_VALUE => {
                let channel = report[1];
                let light = self
                    .lighting
                    .iter_mut()
                    .find(|light| light.channel.id() == channel)
                    .ok_or(Error::Unhandled)?;
                match report[2] {
                    protocol::LIGHT_BRIGHTNESS => light.brightness = report[3],
                    protocol::LIGHT_EFFECT => light.effect = report[3],
                    protocol::LIGHT_SPEED => light.speed = report[3],
                    protocol::LIGHT_COLOR => {
                        light.hue = report[3];
                        light.saturation = report[4];
                    }
                    _ => reply[0] = 0xFF,
                }
            }
            protocol::CUSTOM_SAVE | protocol::EEPROM_RESET | protocol::BOOTLOADER_JUMP => {}
            _ => reply[0] = 0xFF,
        }
        Ok(reply)
    }
}

fn write_u32(reply: &mut Report, value: u32) {
    reply[2..6].copy_from_slice(&value.to_be_bytes());
}

/// A channel the demo can show without a keyboard attached.
pub fn demo_lighting(channel: Channel) -> Lighting {
    Lighting {
        channel,
        brightness: 128,
        effect: 1,
        speed: 128,
        hue: 140,
        saturation: 255,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_keycode_is_what_the_next_read_returns() {
        let map = Keymap::filled(1, 1, 1, 0x0004).unwrap();
        let mut keyboard = MemoryKeyboard::new(map);
        let set = protocol::set_keycode_request(0, 0, 0, 0x0029);
        keyboard.transact(set).unwrap();
        let reply = keyboard
            .transact(protocol::get_keycode_request(0, 0, 0))
            .unwrap();
        assert_eq!(protocol::keycode_from_reply(&reply).unwrap().3, 0x0029);
    }
}
