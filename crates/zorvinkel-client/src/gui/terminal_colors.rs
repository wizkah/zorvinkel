//! Maps terminal colours to egui colours.
use eframe::egui::Color32;

pub const DEFAULT_BACKGROUND: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x27);
pub const DEFAULT_FOREGROUND: Color32 = Color32::from_rgb(0xd8, 0xde, 0xe9);

const BASE_16: [(u8, u8, u8); 16] = [
    (0x1b, 0x1f, 0x27),
    (0xe0, 0x6c, 0x75),
    (0x98, 0xc3, 0x79),
    (0xe5, 0xc0, 0x7b),
    (0x61, 0xaf, 0xef),
    (0xc6, 0x78, 0xdd),
    (0x56, 0xb6, 0xc2),
    (0xd8, 0xde, 0xe9),
    (0x5c, 0x63, 0x70),
    (0xff, 0x7b, 0x86),
    (0xb1, 0xdd, 0x8c),
    (0xf5, 0xd5, 0x8e),
    (0x7c, 0xc0, 0xff),
    (0xd9, 0x8e, 0xed),
    (0x6c, 0xcf, 0xdc),
    (0xff, 0xff, 0xff),
];

pub fn color(color: zorvinkel_terminal::vt100::Color, default: Color32) -> Color32 {
    match color {
        zorvinkel_terminal::vt100::Color::Default => default,
        zorvinkel_terminal::vt100::Color::Rgb(red, green, blue) => {
            Color32::from_rgb(red, green, blue)
        }
        zorvinkel_terminal::vt100::Color::Idx(index) => indexed(index),
    }
}

/// The 256-colour palette: 16 base colours, a 6x6x6 cube and 24 greys.
fn indexed(index: u8) -> Color32 {
    match index {
        0..=15 => {
            let (red, green, blue) = BASE_16[usize::from(index)];
            Color32::from_rgb(red, green, blue)
        }
        16..=231 => {
            let cube = index - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + 40 * step };
            Color32::from_rgb(level(cube / 36), level(cube / 6 % 6), level(cube % 6))
        }
        232..=255 => {
            let grey = 8 + 10 * (index - 232);
            Color32::from_rgb(grey, grey, grey)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_colours_come_from_the_caller() {
        assert_eq!(
            color(
                zorvinkel_terminal::vt100::Color::Default,
                DEFAULT_FOREGROUND
            ),
            DEFAULT_FOREGROUND
        );
    }
    #[test]
    fn cube_and_grey_ramp_cover_the_256_palette() {
        assert_eq!(indexed(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed(196), Color32::from_rgb(255, 0, 0));
        assert_eq!(indexed(232), Color32::from_rgb(8, 8, 8));
        assert_eq!(indexed(255), Color32::from_rgb(238, 238, 238));
    }
    #[test]
    fn true_colour_is_taken_as_is() {
        assert_eq!(
            color(
                zorvinkel_terminal::vt100::Color::Rgb(1, 2, 3),
                DEFAULT_BACKGROUND
            ),
            Color32::from_rgb(1, 2, 3)
        );
    }
}
