//! Colour emoji from bundled Noto Color Emoji (ADR-0013). egui draws glyph
//! outlines in one colour, so emoji are found here and painted as images:
//! the font's cmap and GSUB ligatures give a sequence its glyph (ZWJ
//! families, skin tones, flags, keycaps), and the glyph's PNG from the CBDT
//! table becomes a texture, made once per glyph.

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2};
use ttf_parser::gsub::SubstitutionSubtable;
use ttf_parser::{Face, GlyphId, RasterImageFormat};

static FONT: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji.ttf");

const ZWJ: char = '\u{200d}';
const VS16: char = '\u{fe0f}';
const KEYCAP: char = '\u{20e3}';

fn face() -> Option<&'static Face<'static>> {
    static FACE: OnceLock<Option<Face<'static>>> = OnceLock::new();
    FACE.get_or_init(|| Face::parse(FONT, 0).ok()).as_ref()
}

/// Whether a cell's text should be looked up here rather than drawn by egui: wide
/// characters, emoji presentation (VS16), keycaps and regional indicators. Plain
/// digits and `#` are in the font too, so narrow text needs one of the marks.
pub fn wants(c: char, wide: bool, zerowidth: &[char]) -> bool {
    !c.is_ascii() && (wide || is_regional(c)) || zerowidth.iter().any(|&z| z == VS16 || z == KEYCAP)
}

/// Whether the next cell's character carries on a sequence that ends in `text`.
pub fn continues(text: &str, next: char) -> bool {
    let mut chars = text.chars();
    let lone_regional = matches!((chars.next(), chars.next()), (Some(c), None) if is_regional(c));
    text.ends_with(ZWJ) || is_skin_tone(next) || (lone_regional && is_regional(next))
}

fn is_regional(c: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&c)
}

fn is_skin_tone(c: char) -> bool {
    ('\u{1f3fb}'..='\u{1f3ff}').contains(&c)
}

/// The one glyph `text` becomes, if the font has it.
pub fn glyph(text: &str) -> Option<GlyphId> {
    let face = face()?;
    // Noto's sequences mostly leave VS16 out; try as written first.
    let stripped: String = text.chars().filter(|&c| c != VS16).collect();
    [text, stripped.as_str()].into_iter().find_map(|text| {
        let mut glyphs = text.chars().map(|c| face.glyph_index(c)).collect::<Option<Vec<_>>>()?;
        ligate(face, &mut glyphs);
        match glyphs[..] {
            [one] => Some(one),
            _ => None,
        }
    })
}

/// Applies the font's ligature substitutions, which is all Noto's GSUB holds.
fn ligate(face: &Face, glyphs: &mut Vec<GlyphId>) {
    let Some(gsub) = face.tables().gsub else { return };
    for lookup in gsub.lookups {
        let mut i = 0;
        while i < glyphs.len() {
            let found = lookup.subtables.into_iter::<SubstitutionSubtable>().find_map(|subtable| {
                let SubstitutionSubtable::Ligature(ligatures) = subtable else { return None };
                let set = ligatures.ligature_sets.get(ligatures.coverage.get(glyphs[i])?)?;
                set.into_iter().find(|l| {
                    let rest = &glyphs[i + 1..];
                    l.components.len() as usize <= rest.len() && l.components.into_iter().zip(rest).all(|(a, &b)| a == b)
                })
            });
            if let Some(ligature) = found {
                glyphs.splice(i..=i + ligature.components.len() as usize, [ligature.glyph]);
            }
            i += 1;
        }
    }
}

/// Textures made so far, per glyph; `None` for a glyph with no image.
#[derive(Clone, Default)]
struct Cache(HashMap<GlyphId, Option<TextureHandle>>);

/// Paints `glyph`'s image centred in `rect`, as large as fits.
pub fn paint(painter: &egui::Painter, glyph: GlyphId, rect: Rect) {
    let ctx = painter.ctx();
    let id = egui::Id::new("teleprinter-emoji");
    let texture = ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(id).0.get(&glyph).cloned());
    let texture = texture.unwrap_or_else(|| {
        let made = load(ctx, glyph);
        ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(id).0.insert(glyph, made.clone()));
        made
    });
    let Some(texture) = texture else { return };
    let size = texture.size_vec2();
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    let target = Rect::from_center_size(rect.center(), size * scale);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.image(texture.id(), target, uv, Color32::WHITE);
}

fn load(ctx: &egui::Context, glyph: GlyphId) -> Option<TextureHandle> {
    let raster = face()?.glyph_raster_image(glyph, u16::MAX)?;
    if raster.format != RasterImageFormat::PNG {
        return None;
    }
    let rgba = image::load_from_memory_with_format(raster.data, image::ImageFormat::Png).ok()?.to_rgba8();
    let image = ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba);
    // Drawn at a fraction of the 136 px strike: mipmaps keep it from aliasing.
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    Some(ctx.load_texture(format!("emoji-{}", glyph.0), image, options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_emoji_and_sequences_become_one_glyph() {
        for text in ["🐡", "🐧", "❤\u{fe0f}", "👩\u{200d}👩\u{200d}👧", "👋🏽", "🇸🇪", "1\u{fe0f}\u{20e3}"] {
            assert!(glyph(text).is_some(), "{text:?}");
        }
        assert_ne!(glyph("👋🏽"), glyph("👋"), "the skin tone is its own glyph");
        assert!(glyph("A").is_none() && glyph("中").is_none(), "not emoji");
    }

    #[test]
    fn only_emoji_like_cells_are_looked_up() {
        assert!(wants('🐡', true, &[]));
        assert!(wants('❤', false, &[VS16]));
        assert!(wants('1', false, &[VS16, KEYCAP]));
        assert!(!wants('1', false, &[]), "plain digits stay text");
        assert!(!wants('│', false, &[]));
    }

    #[test]
    fn sequences_span_cells() {
        assert!(continues("👩\u{200d}", '👧'));
        assert!(continues("👋", '🏽'));
        assert!(continues("🇸", '🇪'));
        assert!(!continues("🇸🇪", '🇬'), "flags are pairs");
        assert!(!continues("🐡", '🐧'));
    }
}
