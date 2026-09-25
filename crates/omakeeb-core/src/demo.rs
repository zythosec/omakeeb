//! A 60% ANSI board with a QWERTY layer and a function layer, so the window
//! can be used with no keyboard plugged in.

use crate::error::Result;
use crate::keymap::Keymap;
use crate::layout::Definition;
use crate::lighting::Channel;
use crate::session::{Identity, Session};
use crate::transport::{MemoryKeyboard, demo_lighting};

const DEFINITION: &str = r#"{
  "name": "Demo 60%",
  "vendorId": "0xFEED",
  "productId": "0x6060",
  "matrix": {"rows": 5, "cols": 14},
  "layouts": {
    "keymap": [
      ["0,0","0,1","0,2","0,3","0,4","0,5","0,6","0,7","0,8","0,9","0,10","0,11","0,12",{"w":2},"0,13"],
      [{"w":1.5},"1,0","1,1","1,2","1,3","1,4","1,5","1,6","1,7","1,8","1,9","1,10","1,11","1,12",{"w":1.5},"1,13"],
      [{"w":1.75},"2,0","2,1","2,2","2,3","2,4","2,5","2,6","2,7","2,8","2,9","2,10","2,11",{"w":2.25},"2,12"],
      [{"w":2.25},"3,0","3,1","3,2","3,3","3,4","3,5","3,6","3,7","3,8","3,9","3,10",{"w":2.75},"3,11"],
      [{"w":1.25},"4,0",{"w":1.25},"4,1",{"w":1.25},"4,2",{"w":6.25},"4,3",{"w":1.25},"4,4",{"w":1.25},"4,5",{"w":1.25},"4,6",{"w":1.25},"4,7"]
    ]
  },
  "menus": ["qmk_rgb_matrix"]
}"#;

pub fn definition() -> Definition {
    Definition::parse(DEFINITION).expect("the demo definition is valid")
}

pub fn session() -> Result<Session> {
    let definition = definition();
    let mut keymap = Keymap::filled(4, definition.rows, definition.cols, 0x0001)?;
    // Layer 0 is a QWERTY 60%. Everything else starts transparent.
    let base: &[(u8, u8, u16)] = &[
        (0, 0, 0x0029),
        (0, 1, 0x001E),
        (0, 2, 0x001F),
        (0, 3, 0x0020),
        (0, 4, 0x0021),
        (0, 5, 0x0022),
        (0, 6, 0x0023),
        (0, 7, 0x0024),
        (0, 8, 0x0025),
        (0, 9, 0x0026),
        (0, 10, 0x0027),
        (0, 11, 0x002D),
        (0, 12, 0x002E),
        (0, 13, 0x002A),
        (1, 0, 0x002B),
        (1, 1, 0x0014),
        (1, 2, 0x001A),
        (1, 3, 0x0008),
        (1, 4, 0x0015),
        (1, 5, 0x0017),
        (1, 6, 0x001C),
        (1, 7, 0x0018),
        (1, 8, 0x000C),
        (1, 9, 0x0012),
        (1, 10, 0x0013),
        (1, 11, 0x002F),
        (1, 12, 0x0030),
        (1, 13, 0x0031),
        (2, 0, 0x0039),
        (2, 1, 0x0004),
        (2, 2, 0x0016),
        (2, 3, 0x0007),
        (2, 4, 0x0009),
        (2, 5, 0x000A),
        (2, 6, 0x000B),
        (2, 7, 0x000D),
        (2, 8, 0x000E),
        (2, 9, 0x000F),
        (2, 10, 0x0033),
        (2, 11, 0x0034),
        (2, 12, 0x0028),
        (3, 0, 0x00E1),
        (3, 1, 0x001D),
        (3, 2, 0x001B),
        (3, 3, 0x0006),
        (3, 4, 0x0019),
        (3, 5, 0x0005),
        (3, 6, 0x0011),
        (3, 7, 0x0010),
        (3, 8, 0x0036),
        (3, 9, 0x0037),
        (3, 10, 0x0038),
        (3, 11, 0x00E5),
        (4, 0, 0x00E0),
        (4, 1, 0x00E3),
        (4, 2, 0x00E2),
        (4, 3, 0x002C),
        (4, 4, 0x00E6),
        (4, 5, 0x00E7),
        (4, 6, 0x0065),
        (4, 7, 0x00E4),
    ];
    for &(row, col, code) in base {
        keymap.set(0, row, col, code)?;
    }
    // Layer 1: function keys on the number row, arrows on IJKL, the rest fall through.
    let raised: &[(u8, u8, u16)] = &[
        (0, 1, 0x003A),
        (0, 2, 0x003B),
        (0, 3, 0x003C),
        (0, 4, 0x003D),
        (0, 5, 0x003E),
        (0, 6, 0x003F),
        (0, 7, 0x0040),
        (0, 8, 0x0041),
        (0, 9, 0x0042),
        (0, 10, 0x0043),
        (0, 11, 0x0044),
        (0, 12, 0x0045),
        (1, 7, 0x0050),
        (1, 8, 0x0052),
        (1, 9, 0x0051),
        (1, 10, 0x004F),
        (4, 6, 0x5221),
    ];
    for &(row, col, code) in raised {
        keymap.set(1, row, col, code)?;
    }
    let mut keyboard = MemoryKeyboard::new(keymap);
    keyboard.lighting.push(demo_lighting(Channel::RgbMatrix));
    // One macro to open, so the editor has something in it: a greeting,
    // then Enter.
    let greeting = b"Hello from omakeeb\x01\x01\x28\0";
    keyboard.macro_buffer[..greeting.len()].copy_from_slice(greeting);
    Session::connect(
        Box::new(keyboard),
        Identity {
            path: "demo".into(),
            vendor_id: 0xFEED,
            product_id: 0x6060,
            manufacturer: "omakeeb".into(),
            product: "Demo 60%".into(),
        },
        definition,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_board_is_fifteen_units_wide_and_types_qwerty() {
        let session = session().unwrap();
        let (width, height) = session.definition.bounds();
        assert!((width - 15.0).abs() < 0.01, "{width}");
        assert!((height - 5.0).abs() < 0.01, "{height}");
        let visible = session.definition.visible_keys(0);
        assert_eq!(visible.len(), 61);
        assert_eq!(session.keymap.get(0, 1, 1), Some(0x0014)); // Q
        assert_eq!(session.keymap.get(1, 1, 8), Some(0x0052)); // Up
        assert_eq!(session.keymap.get(1, 1, 1), Some(0x0001)); // transparent
    }
}
