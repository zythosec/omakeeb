//! QMK lighting channels exposed through VIA's custom-value commands.
//!
//! Keymap writes are stored by the firmware on their own. Lighting writes are
//! not: the configurator sends `custom_save` after a change so the value is
//! still there on the next plug-in.

use crate::error::{Error, Result};
use crate::protocol::{self, Report};

/// Which lighting subsystem a definition asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Backlight,
    RgbLight,
    RgbMatrix,
    LedMatrix,
}

impl Channel {
    pub const fn id(self) -> u8 {
        match self {
            Self::Backlight => protocol::CHANNEL_BACKLIGHT,
            Self::RgbLight => protocol::CHANNEL_RGBLIGHT,
            Self::RgbMatrix => protocol::CHANNEL_RGB_MATRIX,
            Self::LedMatrix => protocol::CHANNEL_LED_MATRIX,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Backlight => "Backlight",
            Self::RgbLight => "Underglow",
            Self::RgbMatrix => "RGB matrix",
            Self::LedMatrix => "LED matrix",
        }
    }

    /// Underglow and RGB matrix carry a hue and a saturation. The monochrome
    /// channels do not.
    pub const fn has_color(self) -> bool {
        matches!(self, Self::RgbLight | Self::RgbMatrix)
    }

    pub const fn has_speed(self) -> bool {
        !matches!(self, Self::Backlight)
    }

    pub fn from_menu_token(token: &str) -> Option<Self> {
        match token {
            "qmk_backlight" | "qmk_backlight_rgblight" => Some(Self::Backlight),
            "qmk_rgblight" => Some(Self::RgbLight),
            "qmk_rgb_matrix" => Some(Self::RgbMatrix),
            "qmk_led_matrix" => Some(Self::LedMatrix),
            _ => None,
        }
    }
}

/// The values the panel shows for one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lighting {
    pub channel: Channel,
    pub brightness: u8,
    pub effect: u8,
    pub speed: u8,
    pub hue: u8,
    pub saturation: u8,
}

impl Lighting {
    pub fn read(
        channel: Channel,
        transact: &mut dyn FnMut(Report) -> Result<Report>,
    ) -> Result<Self> {
        let brightness = read_byte(transact, channel, protocol::LIGHT_BRIGHTNESS)?;
        let effect = read_byte(transact, channel, protocol::LIGHT_EFFECT)?;
        let speed = if channel.has_speed() {
            read_byte(transact, channel, protocol::LIGHT_SPEED).unwrap_or(0)
        } else {
            0
        };
        let (hue, saturation) = if channel.has_color() {
            read_color(transact, channel).unwrap_or((0, 255))
        } else {
            (0, 255)
        };
        Ok(Self {
            channel,
            brightness,
            effect,
            speed,
            hue,
            saturation,
        })
    }

    pub fn write_brightness(
        &mut self,
        value: u8,
        transact: &mut dyn FnMut(Report) -> Result<Report>,
    ) -> Result<()> {
        write_byte(transact, self.channel, protocol::LIGHT_BRIGHTNESS, value)?;
        self.brightness = value;
        Ok(())
    }

    pub fn write_effect(
        &mut self,
        value: u8,
        transact: &mut dyn FnMut(Report) -> Result<Report>,
    ) -> Result<()> {
        write_byte(transact, self.channel, protocol::LIGHT_EFFECT, value)?;
        self.effect = value;
        Ok(())
    }

    pub fn write_speed(
        &mut self,
        value: u8,
        transact: &mut dyn FnMut(Report) -> Result<Report>,
    ) -> Result<()> {
        if !self.channel.has_speed() {
            return Err(Error::message("this lighting channel has no speed"));
        }
        write_byte(transact, self.channel, protocol::LIGHT_SPEED, value)?;
        self.speed = value;
        Ok(())
    }

    pub fn write_color(
        &mut self,
        hue: u8,
        saturation: u8,
        transact: &mut dyn FnMut(Report) -> Result<Report>,
    ) -> Result<()> {
        if !self.channel.has_color() {
            return Err(Error::message("this lighting channel has no color"));
        }
        let request =
            protocol::lighting_set(self.channel.id(), protocol::LIGHT_COLOR, &[hue, saturation])?;
        let reply = transact(request)?;
        protocol::ensure_command(&reply, protocol::CUSTOM_SET_VALUE)?;
        save(transact, self.channel)?;
        self.hue = hue;
        self.saturation = saturation;
        Ok(())
    }
}

fn read_byte(
    transact: &mut dyn FnMut(Report) -> Result<Report>,
    channel: Channel,
    value: u8,
) -> Result<u8> {
    let reply = transact(protocol::lighting_get(channel.id(), value))?;
    protocol::lighting_byte(&reply, channel.id(), value)
}

fn read_color(
    transact: &mut dyn FnMut(Report) -> Result<Report>,
    channel: Channel,
) -> Result<(u8, u8)> {
    let reply = transact(protocol::lighting_get(channel.id(), protocol::LIGHT_COLOR))?;
    protocol::lighting_color(&reply, channel.id())
}

fn write_byte(
    transact: &mut dyn FnMut(Report) -> Result<Report>,
    channel: Channel,
    value_id: u8,
    value: u8,
) -> Result<()> {
    let request = protocol::lighting_set(channel.id(), value_id, &[value])?;
    let reply = transact(request)?;
    protocol::ensure_command(&reply, protocol::CUSTOM_SET_VALUE)?;
    save(transact, channel)
}

fn save(transact: &mut dyn FnMut(Report) -> Result<Report>, channel: Channel) -> Result<()> {
    let reply = transact(protocol::lighting_save(channel.id()))?;
    protocol::ensure_command(&reply, protocol::CUSTOM_SAVE)
}

/// A label for the effect index. Firmware builds do not all ship the same list,
/// so the number is the source of truth and the name is the QMK default.
pub fn effect_label(channel: Channel, effect: u8) -> String {
    if effect == 0 {
        return "Off".to_owned();
    }
    let name = match channel {
        Channel::Backlight => match effect {
            1 => Some("On"),
            2 => Some("Breathing"),
            _ => None,
        },
        Channel::RgbLight | Channel::RgbMatrix => RGB_EFFECTS.get(usize::from(effect)).copied(),
        Channel::LedMatrix => LED_EFFECTS.get(usize::from(effect)).copied(),
    };
    match name {
        Some(name) => format!("{effect} {name}"),
        None => format!("Mode {effect}"),
    }
}

const RGB_EFFECTS: [&str; 36] = [
    "Off",
    "Solid",
    "Alphas",
    "Gradient up",
    "Gradient left",
    "Breathing",
    "Band sat",
    "Band val",
    "Pinwheel sat",
    "Pinwheel val",
    "Spiral sat",
    "Spiral val",
    "Cycle all",
    "Cycle left",
    "Cycle up",
    "Chevron",
    "Cycle out",
    "Cycle out dual",
    "Cycle pinwheel",
    "Cycle spiral",
    "Dual beacon",
    "Rainbow beacon",
    "Rainbow pinwheels",
    "Raindrops",
    "Jellybean raindrops",
    "Hue breathing",
    "Hue pendulum",
    "Hue wave",
    "Pixel rain",
    "Pixel flow",
    "Pixel fractal",
    "Typing heatmap",
    "Digital rain",
    "Solid reactive",
    "Solid reactive simple",
    "Solid reactive wide",
];

const LED_EFFECTS: [&str; 12] = [
    "Off",
    "Solid",
    "Alphas",
    "Gradient up",
    "Gradient left",
    "Breathing",
    "Band",
    "Band pinwheel",
    "Band spiral",
    "Cycle left",
    "Cycle up",
    "Cycle out",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_brightness_write_is_followed_by_a_save() {
        let answer = |request: Report, calls: &mut Vec<u8>| {
            calls.push(request[0]);
            let mut reply = request;
            if request[0] == protocol::CUSTOM_GET_VALUE {
                reply[3] = 10;
                reply[4] = 200;
            }
            Ok(reply)
        };
        let mut calls = Vec::new();
        let mut lighting = Lighting::read(Channel::RgbMatrix, &mut |request| {
            answer(request, &mut calls)
        })
        .unwrap();
        assert_eq!(lighting.brightness, 10);
        calls.clear();
        lighting
            .write_brightness(40, &mut |request| answer(request, &mut calls))
            .unwrap();
        assert_eq!(
            calls,
            vec![protocol::CUSTOM_SET_VALUE, protocol::CUSTOM_SAVE]
        );
        assert_eq!(lighting.brightness, 40);
    }

    #[test]
    fn effect_zero_reads_as_off() {
        assert_eq!(effect_label(Channel::RgbMatrix, 0), "Off");
        assert_eq!(effect_label(Channel::RgbMatrix, 1), "1 Solid");
        assert_eq!(effect_label(Channel::RgbMatrix, 200), "Mode 200");
    }
}
