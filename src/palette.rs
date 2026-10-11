//! The terminal's colours: the 16 ANSI colours plus foreground, background,
//! cursor and selection. Each comes from the skin's `[app.teleprinter]`
//! section when it sets one, otherwise from a light or dark default chosen by
//! the current visuals, with the background, text and selection taken from
//! the visuals themselves, so every skin gets a terminal that matches it.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use egui::Color32;
use patina::skin::{Hex, Skin};
use serde::Deserialize;

/// `[app.teleprinter]` in a skin. Every key is optional.
#[derive(Deserialize, Default)]
pub struct SkinColours {
    foreground: Option<Hex>,
    background: Option<Hex>,
    cursor: Option<Hex>,
    selection: Option<Hex>,
    black: Option<Hex>,
    red: Option<Hex>,
    green: Option<Hex>,
    yellow: Option<Hex>,
    blue: Option<Hex>,
    magenta: Option<Hex>,
    cyan: Option<Hex>,
    white: Option<Hex>,
    bright_black: Option<Hex>,
    bright_red: Option<Hex>,
    bright_green: Option<Hex>,
    bright_yellow: Option<Hex>,
    bright_blue: Option<Hex>,
    bright_magenta: Option<Hex>,
    bright_cyan: Option<Hex>,
    bright_white: Option<Hex>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    pub ansi: [Color32; 16],
    pub foreground: Color32,
    pub background: Color32,
    pub cursor: Color32,
    pub selection: Color32,
}

// GNOME Terminal's "Tango" palette, in both variants.
const DARK: [u32; 16] = [
    0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf, //
    0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
];
const LIGHT: [u32; 16] = [
    0x2e3436, 0xcc0000, 0x4e9a06, 0x8f7300, 0x3465a4, 0x75507b, 0x06989a, 0x8a8f86, //
    0x555753, 0xa40000, 0x3d7a00, 0x6b5500, 0x204a87, 0x5c3566, 0x05787a, 0x2e3436,
];

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Palette {
    /// The palette for this skin as it looks right now (Default follows the
    /// desktop), plus any errors or warnings from its `[app.teleprinter]`.
    pub fn new(skin: &Skin, visuals: &egui::Visuals) -> (Self, Vec<String>) {
        let (own, problems) = match skin.app_section::<SkinColours>("teleprinter") {
            Ok((own, warnings)) => (own, warnings),
            Err(error) => (SkinColours::default(), vec![error]),
        };
        let base = if visuals.dark_mode { DARK } else { LIGHT };
        let ansi_keys = [
            &own.black, &own.red, &own.green, &own.yellow, &own.blue, &own.magenta, &own.cyan, &own.white,
            &own.bright_black, &own.bright_red, &own.bright_green, &own.bright_yellow, &own.bright_blue,
            &own.bright_magenta, &own.bright_cyan, &own.bright_white,
        ];
        let ansi = std::array::from_fn(|i| ansi_keys[i].as_ref().map_or(rgb(base[i]), |h| h.0));
        let pick = |key: &Option<Hex>, fallback: Color32| key.as_ref().map_or(fallback, |h| h.0);
        let palette = Self {
            ansi,
            foreground: pick(&own.foreground, visuals.strong_text_color()),
            background: pick(&own.background, visuals.extreme_bg_color),
            cursor: pick(&own.cursor, visuals.strong_text_color()),
            selection: pick(&own.selection, visuals.selection.bg_fill),
        };
        (palette, problems)
    }

    /// A cell colour, honouring colours the program changed with OSC 4/10/11.
    pub fn resolve(&self, color: Color, overrides: &Colors) -> Color32 {
        match color {
            Color::Spec(c) => Color32::from_rgb(c.r, c.g, c.b),
            Color::Indexed(i) => self.indexed(i as usize, overrides),
            Color::Named(name) => self.named(name, overrides),
        }
    }

    pub fn named(&self, name: NamedColor, overrides: &Colors) -> Color32 {
        if let Some(c) = overrides[name] {
            return Color32::from_rgb(c.r, c.g, c.b);
        }
        let index = name as usize;
        match name {
            _ if index < 16 => self.ansi[index],
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            // DimBlack..DimWhite follow Cursor, in ANSI order.
            _ => dim(self.ansi[index - NamedColor::DimBlack as usize]),
        }
    }

    fn indexed(&self, index: usize, overrides: &Colors) -> Color32 {
        if let Some(c) = overrides[index] {
            return Color32::from_rgb(c.r, c.g, c.b);
        }
        match index {
            0..16 => self.ansi[index],
            16..232 => {
                // The 6×6×6 colour cube.
                let i = index - 16;
                let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
                Color32::from_rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
            }
            _ => {
                let v = (8 + (index - 232) * 10) as u8;
                Color32::from_gray(v)
            }
        }
    }

    /// The RGB a program gets back when it asks for colour `index` (OSC 4/10/11 queries).
    pub fn query(&self, index: usize, overrides: &Colors) -> Rgb {
        let c = if index < 256 {
            self.indexed(index, overrides)
        } else {
            match index {
                256 => self.foreground,
                257 => self.background,
                _ => self.cursor,
            }
        };
        Rgb { r: c.r(), g: c.g(), b: c.b() }
    }
}

pub fn dim(c: Color32) -> Color32 {
    Color32::from_rgb((c.r() as f32 * 0.66) as u8, (c.g() as f32 * 0.66) as u8, (c.b() as f32 * 0.66) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cube_and_greys_match_xterm() {
        let p = Palette::new(&patina::skin::load_all(patina::skin::BUILT_IN, &[]).0[0], &egui::Visuals::dark()).0;
        let none = Colors::default();
        assert_eq!(p.resolve(Color::Indexed(16), &none), Color32::from_rgb(0, 0, 0));
        assert_eq!(p.resolve(Color::Indexed(196), &none), Color32::from_rgb(255, 0, 0));
        assert_eq!(p.resolve(Color::Indexed(231), &none), Color32::from_rgb(255, 255, 255));
        assert_eq!(p.resolve(Color::Indexed(232), &none), Color32::from_gray(8));
        assert_eq!(p.resolve(Color::Indexed(255), &none), Color32::from_gray(238));
    }

    #[test]
    fn a_skin_sets_its_own_terminal_colours() {
        let skin = Skin::parse("format = 1\nname = \"T\"\n[app.teleprinter]\nred = \"#ff0000\"\nbackgroud = \"#000000\"")
            .unwrap();
        let (p, problems) = Palette::new(&skin, &egui::Visuals::dark());
        assert_eq!(p.ansi[1], Color32::from_rgb(255, 0, 0));
        assert_eq!(p.ansi[2], rgb(DARK[2]));
        assert_eq!(problems, ["unknown key `app.teleprinter.backgroud` ignored"]);
    }

    #[test]
    fn the_background_follows_the_visuals() {
        let skin = &patina::skin::load_all(patina::skin::BUILT_IN, &[]).0[0];
        let light = egui::Visuals::light();
        assert_eq!(Palette::new(skin, &light).0.background, light.extreme_bg_color);
    }
}
