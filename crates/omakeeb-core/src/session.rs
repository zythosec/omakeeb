//! One connected keyboard: its definition, the keymap, and the writes.

use crate::error::{Error, Result};
use crate::keymap::{self, Keymap};
use crate::layout::Definition;
use crate::lighting::Lighting;
use crate::macros::{Action, Macros};
use crate::protocol::{self, Report};
use crate::transport::Link;

/// Identity copied off the USB device, or supplied for the demo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: String,
    pub product: String,
}

/// The keyboard the window is editing.
pub struct Session {
    link: Box<dyn Link>,
    pub identity: Identity,
    pub protocol: u16,
    pub firmware: u32,
    pub definition: Definition,
    pub keymap: Keymap,
    pub layout_options: u32,
    pub lighting: Vec<Lighting>,
    /// `None` when the firmware has no dynamic macros.
    pub macros: Option<Macros>,
    pub demo: bool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("identity", &self.identity)
            .field("protocol", &self.protocol)
            .field("firmware", &self.firmware)
            .field("definition", &self.definition.name)
            .field("demo", &self.demo)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Read protocol, keymap and lighting. `definition` supplies the matrix;
    /// the protocol does not.
    pub fn connect(
        mut link: Box<dyn Link>,
        identity: Identity,
        definition: Definition,
        demo: bool,
    ) -> Result<Self> {
        let protocol = protocol::protocol_version(
            &link.transact(protocol::blank(protocol::GET_PROTOCOL_VERSION))?,
        )?;
        let layers = match link.transact(protocol::layer_count_request()) {
            Ok(reply) => protocol::layer_count(&reply).or(Ok(4))?,
            Err(Error::Unhandled) => 4,
            Err(err) => return Err(err),
        };
        let keymap = keymap::read_from_device(
            &mut |report| link.transact(report),
            layers,
            definition.rows,
            definition.cols,
        )?;
        let layout_options = match link.transact(protocol::u32_value_request(
            protocol::GET_KEYBOARD_VALUE,
            protocol::VALUE_LAYOUT_OPTIONS,
        )) {
            Ok(reply) => protocol::u32_from_value(
                &reply,
                protocol::GET_KEYBOARD_VALUE,
                protocol::VALUE_LAYOUT_OPTIONS,
            )
            .unwrap_or(0),
            Err(Error::Unhandled) => 0,
            Err(err) => return Err(err),
        };
        let firmware = match link.transact(protocol::u32_value_request(
            protocol::GET_KEYBOARD_VALUE,
            protocol::VALUE_FIRMWARE_VERSION,
        )) {
            Ok(reply) => protocol::u32_from_value(
                &reply,
                protocol::GET_KEYBOARD_VALUE,
                protocol::VALUE_FIRMWARE_VERSION,
            )
            .unwrap_or(0),
            Err(Error::Unhandled) => 0,
            Err(err) => return Err(err),
        };
        let mut lighting = Vec::new();
        for channel in &definition.lighting {
            let channel = *channel;
            match Lighting::read(channel, &mut |report| link.transact(report)) {
                Ok(value) => lighting.push(value),
                Err(Error::Unhandled) => {}
                Err(err) => return Err(err),
            }
        }
        let macros = read_macros(&mut *link)?;
        // Ask the board to blink, the way VIA does when a device becomes the
        // one being configured. A firmware that ignores it is still usable.
        let _ = link.transact(protocol::set_u32_value(
            protocol::VALUE_DEVICE_INDICATION,
            1,
        ));
        Ok(Self {
            link,
            identity,
            protocol,
            firmware,
            definition,
            keymap,
            layout_options,
            lighting,
            macros,
            demo,
        })
    }

    /// Replace macro `index` and write every macro back, since they share
    /// one buffer.
    pub fn set_macro(&mut self, index: u8, actions: Vec<Action>) -> Result<()> {
        let macros = self
            .macros
            .as_mut()
            .ok_or_else(|| Error::message("this keyboard has no dynamic macros"))?;
        macros.set(index, actions, &mut |report| self.link.transact(report))
    }

    pub fn set_keycode(&mut self, layer: u8, row: u8, col: u8, code: u16) -> Result<()> {
        let reply = self
            .link
            .transact(protocol::set_keycode_request(layer, row, col, code))?;
        protocol::ensure_command(&reply, protocol::DYNAMIC_KEYMAP_SET_KEYCODE)?;
        self.keymap.set(layer, row, col, code)
    }

    pub fn set_layout_options(&mut self, value: u32) -> Result<()> {
        let reply = self.link.transact(protocol::set_u32_value(
            protocol::VALUE_LAYOUT_OPTIONS,
            value,
        ))?;
        protocol::ensure_command(&reply, protocol::SET_KEYBOARD_VALUE)?;
        self.layout_options = value;
        Ok(())
    }

    pub fn reset_keymap(&mut self) -> Result<()> {
        let reply = self
            .link
            .transact(protocol::blank(protocol::DYNAMIC_KEYMAP_RESET))?;
        protocol::ensure_command(&reply, protocol::DYNAMIC_KEYMAP_RESET)?;
        self.keymap = keymap::read_from_device(
            &mut |report| self.link.transact(report),
            self.keymap.layers,
            self.definition.rows,
            self.definition.cols,
        )?;
        Ok(())
    }

    pub fn reset_eeprom(&mut self) -> Result<()> {
        let reply = self
            .link
            .transact(protocol::blank(protocol::EEPROM_RESET))?;
        protocol::ensure_command(&reply, protocol::EEPROM_RESET)
    }

    pub fn jump_bootloader(&mut self) -> Result<()> {
        let reply = self
            .link
            .transact(protocol::blank(protocol::BOOTLOADER_JUMP))?;
        protocol::ensure_command(&reply, protocol::BOOTLOADER_JUMP)
    }

    pub fn adjust_brightness(&mut self, index: usize, delta: i16) -> Result<()> {
        let current = self
            .lighting
            .get(index)
            .ok_or_else(|| Error::message("no lighting"))?
            .brightness;
        let next = add_u8(current, delta);
        self.write_light(index, |light, link| light.write_brightness(next, link))
    }

    pub fn adjust_effect(&mut self, index: usize, delta: i16) -> Result<()> {
        let current = self
            .lighting
            .get(index)
            .ok_or_else(|| Error::message("no lighting"))?
            .effect;
        let next = add_u8(current, delta);
        self.write_light(index, |light, link| light.write_effect(next, link))
    }

    pub fn adjust_speed(&mut self, index: usize, delta: i16) -> Result<()> {
        let current = self
            .lighting
            .get(index)
            .ok_or_else(|| Error::message("no lighting"))?
            .speed;
        let next = add_u8(current, delta);
        self.write_light(index, |light, link| light.write_speed(next, link))
    }

    pub fn adjust_hue(&mut self, index: usize, delta: i16) -> Result<()> {
        let light = self
            .lighting
            .get(index)
            .ok_or_else(|| Error::message("no lighting"))?;
        let hue = add_u8(light.hue, delta);
        let saturation = light.saturation;
        self.write_light(index, move |light, link| {
            light.write_color(hue, saturation, link)
        })
    }

    pub fn adjust_saturation(&mut self, index: usize, delta: i16) -> Result<()> {
        let light = self
            .lighting
            .get(index)
            .ok_or_else(|| Error::message("no lighting"))?;
        let hue = light.hue;
        let saturation = add_u8(light.saturation, delta);
        self.write_light(index, move |light, link| {
            light.write_color(hue, saturation, link)
        })
    }

    fn write_light(
        &mut self,
        index: usize,
        write: impl FnOnce(&mut Lighting, &mut dyn FnMut(Report) -> Result<Report>) -> Result<()>,
    ) -> Result<()> {
        let mut light = self
            .lighting
            .get(index)
            .copied()
            .ok_or_else(|| Error::message("no lighting"))?;
        write(&mut light, &mut |report| self.link.transact(report))?;
        self.lighting[index] = light;
        Ok(())
    }
}

/// Count and buffer size come first; a firmware without either, or with
/// neither macros nor room, has no macros to edit.
fn read_macros(link: &mut dyn Link) -> Result<Option<Macros>> {
    let count = match link.transact(protocol::blank(protocol::MACRO_GET_COUNT)) {
        Ok(reply) => match protocol::macro_count(&reply) {
            Ok(count) => count,
            Err(Error::Unhandled) => return Ok(None),
            Err(err) => return Err(err),
        },
        Err(Error::Unhandled) => return Ok(None),
        Err(err) => return Err(err),
    };
    let size = match link.transact(protocol::blank(protocol::MACRO_GET_BUFFER_SIZE)) {
        Ok(reply) => match protocol::macro_buffer_size(&reply) {
            Ok(size) => size,
            Err(Error::Unhandled) => return Ok(None),
            Err(err) => return Err(err),
        },
        Err(Error::Unhandled) => return Ok(None),
        Err(err) => return Err(err),
    };
    if count == 0 || size == 0 {
        return Ok(None);
    }
    Macros::read_from_device(&mut |report| link.transact(report), count, size).map(Some)
}

fn add_u8(value: u8, delta: i16) -> u8 {
    let next = i16::from(value) + delta;
    next.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Definition;
    use crate::transport::MemoryKeyboard;

    fn fixture() -> (Definition, MemoryKeyboard) {
        let definition = Definition::parse(
            r#"{
                "name": "Pad",
                "vendorId": "0xFEED",
                "productId": "0x0060",
                "matrix": {"rows": 1, "cols": 2},
                "layouts": {"keymap": [["0,0", "0,1"]]},
                "menus": ["qmk_rgb_matrix"]
            }"#,
        )
        .unwrap();
        let mut keyboard = MemoryKeyboard::new(Keymap::filled(2, 1, 2, 0x0004).unwrap());
        keyboard.lighting.push(crate::transport::demo_lighting(
            crate::lighting::Channel::RgbMatrix,
        ));
        (definition, keyboard)
    }

    #[test]
    fn connecting_reads_the_keymap_and_a_write_sticks() {
        let (definition, keyboard) = fixture();
        let mut session = Session::connect(
            Box::new(keyboard),
            Identity {
                path: "memory".into(),
                vendor_id: 0xFEED,
                product_id: 0x0060,
                manufacturer: "Test".into(),
                product: "Pad".into(),
            },
            definition,
            true,
        )
        .unwrap();
        assert_eq!(session.protocol, 13);
        assert_eq!(session.keymap.get(0, 0, 1), Some(0x0004));
        assert_eq!(session.lighting.len(), 1);
        session.set_keycode(1, 0, 1, 0x0029).unwrap();
        assert_eq!(session.keymap.get(1, 0, 1), Some(0x0029));
        assert_eq!(session.keymap.get(0, 0, 0), Some(0x0004));
        session.adjust_brightness(0, 10).unwrap();
        assert_eq!(session.lighting[0].brightness, 138);
    }

    #[test]
    fn a_macro_edit_is_written_and_read_back() {
        let (definition, keyboard) = fixture();
        let identity = Identity {
            path: "memory".into(),
            vendor_id: 0xFEED,
            product_id: 0x0060,
            manufacturer: "Test".into(),
            product: "Pad".into(),
        };
        let mut session = Session::connect(
            Box::new(keyboard),
            identity.clone(),
            definition.clone(),
            true,
        )
        .unwrap();
        assert_eq!(session.macros.as_ref().map(Macros::count), Some(16));
        let typed = crate::macros::from_text("hi{KC_ENTER}").unwrap();
        session.set_macro(1, typed.clone()).unwrap();
        assert!(session.set_macro(16, Vec::new()).is_err());
        let Session { link, .. } = session;
        let again = Session::connect(link, identity, definition, true).unwrap();
        assert_eq!(again.macros.unwrap().macros[1], typed);
    }

    #[test]
    fn a_keyboard_without_macros_connects_without_them() {
        let (definition, mut keyboard) = fixture();
        keyboard.macro_count = 0;
        let session = Session::connect(
            Box::new(keyboard),
            Identity {
                path: "memory".into(),
                vendor_id: 0xFEED,
                product_id: 0x0060,
                manufacturer: "Test".into(),
                product: "Pad".into(),
            },
            definition,
            true,
        )
        .unwrap();
        assert!(session.macros.is_none());
    }
}
