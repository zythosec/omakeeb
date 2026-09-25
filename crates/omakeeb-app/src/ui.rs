//! Spacing and type, on one rem scale.
//!
//! The same rule disktree follows: a size is chosen by what two things mean
//! to each other, and interface zoom keeps the relationship. Corners stay
//! square, which is how Omarchy draws a surface. Pixels are only for the
//! keyboard, whose geometry comes from the key layout rather than from type.

use gpui_kit::Rems;

pub mod space {
    use super::Rems;

    pub const XS: Rems = Rems(0.25);
    pub const SM: Rems = Rems(0.5);
    pub const MD: Rems = Rems(0.75);
    pub const LG: Rems = Rems(1.0);
    pub const XL: Rems = Rems(1.5);
}

pub mod text {
    use super::Rems;

    pub const CAPTION: Rems = Rems(0.6875);
    pub const BODY: Rems = Rems(0.75);
    pub const TITLE: Rems = Rems(0.875);
    pub const HEADING: Rems = Rems(1.125);
    pub const DISPLAY: Rems = Rems(2.5);
}

pub mod size {
    use super::Rems;

    pub const PANEL: Rems = Rems(22.0);
    pub const HELP: Rems = Rems(36.0);
    pub const KEY_LANE: Rems = Rems(7.5);
    /// The least height the keycode list gets, about eight rows. Beside the
    /// board it grows past this to fill the panel; in a window too short for
    /// that, the panel scrolls rather than the list shrinking to nothing.
    pub const PICKER_LIST_MIN: Rems = Rems(12.0);
    pub const BAR: Rems = Rems(2.75);
    /// The mark beside the title. It is drawn on a 24-unit grid, so at 100%
    /// zoom every edge lands on a whole pixel; at the title's 18px it blurs.
    pub const MARK: Rems = Rems(1.5);
    pub const STATUS: Rems = Rems(1.125);

    /// Narrower than this, the side panel moves under the keyboard. A 60%
    /// board at a legible key (15 × `KEY_MIN`), its padding and the panel
    /// need about this much side by side.
    pub const STACK_BELOW: Rems = Rems(56.0);
    /// The smallest key. A caption legend of four or five characters still
    /// fits inside it; below it legends collide, so the board scrolls instead.
    pub const KEY_MIN: Rems = Rems(2.0);
    /// The largest key, so a small board in a big window stays keyboard-sized.
    pub const KEY_MAX: Rems = Rems(5.5);
    /// Stacked, the board takes at most this share of the height, so the
    /// panel's first rows show below it without scrolling.
    pub const STACKED_BOARD_SHARE: f32 = 0.6;
    /// Stacked, the keycode list may take this share of the height. Scrolled
    /// to, it fills most of the view and still leaves its heading showing.
    pub const STACKED_LIST_SHARE: f32 = 0.7;
}

/// Interface zoom as a factor of the 16px rem. Index 2 is 100%.
pub const ZOOM: [f32; 7] = [0.75, 0.875, 1.0, 1.125, 1.25, 1.5, 1.75];
pub const ZOOM_HOME: usize = 2;
pub const BASE_REM: f32 = 16.0;
