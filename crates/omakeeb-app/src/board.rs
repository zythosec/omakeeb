//! The keyboard, drawn from the definition in key units and scaled to the window.

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    Window, div, px, relative,
};
use gpui_omarchy::{ActiveTheme, Theme};
use omakeeb_core::{Definition, Keymap, is_layer_switch, short_name};

use crate::app::Omakeeb;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Transparent,
    Empty,
    Layer,
    Decal,
}

pub struct Cap {
    pub selectable: Option<usize>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub x2: f32,
    pub y2: f32,
    pub w2: f32,
    pub h2: f32,
    pub label: SharedString,
    pub tone: Tone,
}

/// Selectable keys, in key units, for the layout options currently stored.
pub fn caps(definition: &Definition, keymap: &Keymap, layer: u8, options: u32) -> Vec<Cap> {
    let mut index = 0;
    definition
        .visible_keys(options)
        .into_iter()
        .map(|key| {
            let selectable = key.selectable().then(|| {
                let current = index;
                index += 1;
                current
            });
            let code = key
                .row
                .zip(key.col)
                .and_then(|(row, col)| keymap.get(layer, row, col));
            let (label, tone) = match (key.decal, code) {
                (true, _) => (SharedString::from(key.legend.clone()), Tone::Decal),
                (_, Some(0x0000)) => (SharedString::from(""), Tone::Empty),
                (_, Some(0x0001)) => (SharedString::from("▽"), Tone::Transparent),
                (_, Some(code)) if is_layer_switch(code) => (
                    SharedString::from(short_name(code, &definition.custom)),
                    Tone::Layer,
                ),
                (_, Some(code)) => (
                    SharedString::from(short_name(code, &definition.custom)),
                    Tone::Normal,
                ),
                (_, None) => (SharedString::from(key.legend.clone()), Tone::Decal),
            };
            Cap {
                selectable,
                x: key.x,
                y: key.y,
                w: key.w,
                h: key.h,
                x2: key.x2,
                y2: key.y2,
                w2: key.w2,
                h2: key.h2,
                label,
                tone,
            }
        })
        .collect()
}

pub fn keyboard(
    caps: &[Cap],
    unit: f32,
    width: f32,
    height: f32,
    selected: usize,
    window: &Window,
    cx: &mut Context<'_, Omakeeb>,
) -> impl IntoElement {
    let theme = cx.omarchy().clone();
    let rem = pixels(window.rem_size());
    let gap = unit * 0.055;
    div()
        .relative()
        // Fixed, so a board wider than its scroll container overflows rather
        // than being squeezed: its keys are positioned absolutely and would
        // otherwise let it shrink to nothing.
        .flex_shrink_0()
        .w(px(width))
        .h(px(height))
        .children(caps.iter().enumerate().flat_map(|(nth, cap)| {
            let mut parts = Vec::new();
            if cap.w2 > 0.0 && cap.h2 > 0.0 {
                parts.push(key_rect(
                    format!("key-{nth}-step"),
                    cap.x + cap.x2,
                    cap.y + cap.y2,
                    cap.w2,
                    cap.h2,
                    unit,
                    gap,
                    cap,
                    false,
                    selected,
                    &theme,
                    rem,
                    cx,
                ));
            }
            parts.push(key_rect(
                format!("key-{nth}"),
                cap.x,
                cap.y,
                cap.w,
                cap.h,
                unit,
                gap,
                cap,
                true,
                selected,
                &theme,
                rem,
                cx,
            ));
            parts
        }))
}

fn key_rect(
    id: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    unit: f32,
    gap: f32,
    cap: &Cap,
    labeled: bool,
    selected: usize,
    theme: &Theme,
    rem: f32,
    cx: &mut Context<'_, Omakeeb>,
) -> gpui_kit::AnyElement {
    let selected_cap = cap.selectable == Some(selected);
    let (fill, fg) = colors(cap.tone, selected_cap, theme);
    let border = if selected_cap {
        theme.warning
    } else {
        theme.border
    };
    let width = (w * unit - gap * 2.0).max(4.0);
    let height = (h * unit - gap * 2.0).max(4.0);
    let text = if labeled {
        cap.label.clone()
    } else {
        SharedString::from("")
    };
    let narrow = w < 1.35 || text.len() > 5;
    let rect = div()
        .id(SharedString::from(id))
        .absolute()
        .left(px(x * unit + gap))
        .top(px(y * unit + gap))
        .w(px(width))
        .h(px(height))
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .px(px((rem * 0.125).min(gap)))
        .border_1()
        .border_color(border)
        .bg(fill)
        .text_color(fg)
        .font_family(theme.mono_font.clone())
        .text_size(if narrow {
            crate::ui::text::CAPTION
        } else {
            crate::ui::text::BODY
        })
        .line_height(relative(1.05))
        .child(text);
    if let Some(index) = cap.selectable {
        rect.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.select_key(index, window, cx);
            }),
        )
        .into_any_element()
    } else {
        rect.into_any_element()
    }
}

fn colors(tone: Tone, selected: bool, theme: &Theme) -> (gpui_kit::Hsla, gpui_kit::Hsla) {
    let fill = match tone {
        Tone::Normal => theme.surface,
        Tone::Transparent => theme.inset,
        Tone::Empty => theme.background,
        Tone::Layer => theme.accent.opacity(0.16),
        Tone::Decal => theme.background,
    };
    let fg = match tone {
        Tone::Transparent | Tone::Empty | Tone::Decal => theme.secondary,
        Tone::Normal | Tone::Layer => {
            if selected {
                theme.bright
            } else {
                theme.foreground
            }
        }
    };
    (fill, fg)
}

pub fn pixels(value: gpui_kit::Pixels) -> f32 {
    f32::from(value)
}

/// The scroll position, along one axis, that shows the span `start..start +
/// len` inside a view of `view` pixels, moving as little as possible from
/// `scroll`. A span larger than the view shows its start.
pub fn reveal(scroll: f32, view: f32, start: f32, len: f32) -> f32 {
    if start < scroll || len > view {
        start
    } else if start + len > scroll + view {
        start + len - view
    } else {
        scroll
    }
}

/// Nearest selectable cap in `direction`, which is a unit vector.
pub fn neighbor(caps: &[Cap], selected: usize, direction: (f32, f32)) -> Option<usize> {
    let origin = caps.iter().find(|cap| cap.selectable == Some(selected))?;
    let (ox, oy) = (origin.x + origin.w / 2.0, origin.y + origin.h / 2.0);
    let mut best: Option<(usize, f32)> = None;
    for cap in caps {
        let Some(index) = cap.selectable else {
            continue;
        };
        if index == selected {
            continue;
        }
        let dx = (cap.x + cap.w / 2.0) - ox;
        let dy = (cap.y + cap.h / 2.0) - oy;
        let distance = dx.hypot(dy);
        if distance < 0.05 {
            continue;
        }
        let alignment = (dx * direction.0 + dy * direction.1) / distance;
        if alignment < 0.4 {
            continue;
        }
        let score = distance / alignment;
        if best.is_none_or(|(_, previous)| score < previous) {
            best = Some((index, score));
        }
    }
    best.map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::reveal;

    #[test]
    fn reveal_moves_only_as_far_as_needed() {
        // Already visible: stay put.
        assert!((reveal(100.0, 300.0, 150.0, 40.0) - 100.0).abs() < f32::EPSILON);
        // Off the start: its start becomes the view's start.
        assert!((reveal(100.0, 300.0, 60.0, 40.0) - 60.0).abs() < f32::EPSILON);
        // Off the end: its end becomes the view's end.
        assert!((reveal(100.0, 300.0, 380.0, 40.0) - 120.0).abs() < f32::EPSILON);
        // Wider than the view: show where it starts.
        assert!((reveal(0.0, 30.0, 80.0, 40.0) - 80.0).abs() < f32::EPSILON);
    }
}
