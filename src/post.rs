//! Post-processing: resize to exactly 3840x2160 (cover + center crop) and
//! stamp the file name in the bottom-right corner. Pure Rust: the `image`
//! crate does the scaling and `ab_glyph` rasterises a bundled font, so no
//! external tools are needed on any platform.

use std::path::Path;

use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use image::{DynamicImage, ImageBuffer, Pixel};

pub const TARGET_WIDTH: u32 = 3840;
pub const TARGET_HEIGHT: u32 = 2160;

/// Label size in pixels (matches ImageMagick's former `-pointsize 48`).
const LABEL_PX: f32 = 48.0;
/// Distance of the label from the right edge and bottom edge, in pixels.
const MARGIN_RIGHT: f32 = 60.0;
const MARGIN_BOTTOM: f32 = 120.0;
/// Soft dark shadow offset behind the white text, so the label stays readable
/// over bright nebulosity.
const SHADOW_OFFSET: f32 = 2.0;
const SHADOW_OPACITY: f32 = 0.7;

/// DejaVu Sans Condensed Bold - a close, freely redistributable stand-in for
/// the condensed gothic look originally intended. License: assets/fonts/LICENSE-DejaVu.txt
static BUNDLED_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSansCondensed-Bold.ttf");

/// Holds the label font; create once and reuse for every image.
pub struct Stamper {
    font: FontArc,
}

impl Stamper {
    /// Use the bundled font.
    pub fn bundled() -> Self {
        let font = FontArc::try_from_slice(BUNDLED_FONT).expect("bundled font is valid");
        Stamper { font }
    }

    /// Use a TrueType / OpenType font file supplied by the user.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("cannot read font file {}: {e}", path.display()))?;
        let font = FontArc::try_from_vec(bytes)
            .map_err(|_| format!("{} is not a valid TrueType/OpenType font", path.display()))?;
        Ok(Stamper { font })
    }

    /// Scale `img` (up or down, aspect ratio kept) so it covers 3840x2160,
    /// center-crop the overflow to exactly 3840x2160, and draw `label` in the
    /// bottom-right corner. Mono images stay mono; RGB stays RGB; anything
    /// else is converted to RGB.
    pub fn resize_and_label(&self, img: DynamicImage, label: &str) -> DynamicImage {
        let resized = img.resize_to_fill(
            TARGET_WIDTH,
            TARGET_HEIGHT,
            image::imageops::FilterType::Lanczos3,
        );

        match resized {
            DynamicImage::ImageLuma8(mut buf) => {
                self.draw_label(&mut buf, label);
                DynamicImage::ImageLuma8(buf)
            }
            DynamicImage::ImageRgb8(mut buf) => {
                self.draw_label(&mut buf, label);
                DynamicImage::ImageRgb8(buf)
            }
            other => {
                let mut buf = other.to_rgb8();
                self.draw_label(&mut buf, label);
                DynamicImage::ImageRgb8(buf)
            }
        }
    }

    /// Draw `text` right-aligned in the bottom-right corner: white with a soft
    /// dark shadow.
    fn draw_label<P>(&self, img: &mut ImageBuffer<P, Vec<u8>>, text: &str)
    where
        P: Pixel<Subpixel = u8>,
    {
        if text.is_empty() {
            return;
        }
        let scale = PxScale::from(LABEL_PX);
        let scaled = self.font.as_scaled(scale);

        // Lay out glyphs left-to-right from x = 0 to measure the total width.
        let mut glyphs = Vec::with_capacity(text.len());
        let mut x = 0.0f32;
        let mut prev = None;
        for ch in text.chars() {
            let id = self.font.glyph_id(ch);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            glyphs.push((id, x));
            x += scaled.h_advance(id);
            prev = Some(id);
        }
        let text_width = x;

        let right = img.width() as f32 - MARGIN_RIGHT;
        let x0 = right - text_width;
        let baseline = img.height() as f32 - MARGIN_BOTTOM - scaled.descent().abs();

        // Shadow first, then the white text on top.
        let origin = point(x0, baseline);
        let shadow_origin = point(x0 + SHADOW_OFFSET, baseline + SHADOW_OFFSET);
        self.blit(img, &glyphs, scale, shadow_origin, 0, SHADOW_OPACITY);
        self.blit(img, &glyphs, scale, origin, 255, 1.0);
    }

    /// Rasterise `glyphs` (id + x offset from `origin`, whose y is the
    /// baseline) into `img`, blending every channel towards `value`.
    fn blit<P>(
        &self,
        img: &mut ImageBuffer<P, Vec<u8>>,
        glyphs: &[(ab_glyph::GlyphId, f32)],
        scale: PxScale,
        origin: ab_glyph::Point,
        value: u8,
        opacity: f32,
    ) where
        P: Pixel<Subpixel = u8>,
    {
        let (w, h) = (img.width() as i64, img.height() as i64);
        for &(id, dx) in glyphs {
            let glyph = id.with_scale_and_position(scale, point(origin.x + dx, origin.y));
            let Some(outlined) = self.font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i64 + gx as i64;
                let py = bounds.min.y as i64 + gy as i64;
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let a = (coverage * opacity).clamp(0.0, 1.0);
                if a <= 0.0 {
                    return;
                }
                let pixel = img.get_pixel_mut(px as u32, py as u32);
                for c in pixel.channels_mut() {
                    let src = *c as f32;
                    *c = (src + (value as f32 - src) * a).round().clamp(0.0, 255.0) as u8;
                }
            });
        }
    }
}
