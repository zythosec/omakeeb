//! The keyboard half of omakeeb, with no window in it.
//!
//! A definition says where the keys are. The VIA protocol says what they do.
//! The session is the only place that writes, and it will not write a key
//! outside the matrix the definition declared.

pub mod catalog;
pub mod demo;
pub mod error;
pub mod hid;
pub mod keycode;
pub mod keymap;
pub mod layout;
pub mod lighting;
pub mod macros;
pub mod protocol;
pub mod session;
pub mod transport;

pub use catalog::{
    Catalog, definitions_dir, ensure_dir, file_name, keymaps_dir, remember, remember_in,
};
pub use demo::session as demo_session;
pub use error::{Error, Result};
pub use hid::{Discovered, HidLink, discover};
pub use keycode::{
    Group, Item, is_layer_switch, items_for, long_name, macro_code, macro_index,
    parse as parse_keycode, short_name,
};
pub use keymap::Keymap;
pub use layout::{Definition, Key, LayoutGroup};
pub use lighting::{Channel, Lighting, effect_label};
pub use macros::{Action, Macros};
pub use session::{Identity, Session};
pub use transport::{Link, MemoryKeyboard};
