//! CPU glyph rasterization for MITOS shell text.
//!
//! Renders strings into RGBA images using a system TTF font, then
//! uploads them through the same MemoryRenderBuffer path as the
//! wallpaper. No new GPU code required.

use ab_glyph::{Font, FontArc, Glyph, PxScale, ScaleFont, point};
use image::RgbaImage;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer,
    MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::gles::{GlesError, GlesRenderer};
use smithay::utils::{Logical, Rectangle, Size, Transform};

/// Font search order: MITOS font first, then common Linux paths.
const FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/mitos/MitosSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
];

pub struct TextRenderer {
    font: Option<FontArc>,
}

impl TextRenderer {
    pub fn new() -> Self {
        let mut font = None;

        for path in FONT_PATHS {
            let Ok(data) = std::fs::read(path) else { continue };

            if let Ok(f) = FontArc::try_from_vec(data) {
                tracing::info!("MITOS GUI: font loaded from {path}");
                font = Some(f);
                break;
            }
        }

        if font.is_none() {
            tracing::warn!("MITOS GUI: no system font found, text disabled");
        }

        Self { font }
    }

    /// Rasterize one line of text into an RGBA image.
    pub fn render(
        &self,
        text: &str,
        size: f32,
        rgba: (u8, u8, u8, u8),
    ) -> Option<RgbaImage> {
        let font = self.font.as_ref()?;

        if text.is_empty() {
            return None;
        }

        let scale = PxScale::from(size);
        let scaled = font.as_scaled(scale);

        // --------------------------------------------------------
        // Layout: horizontal advance + kerning
        // --------------------------------------------------------
        let mut x: f32 = 0.0;
        let mut prev = None;
        let mut glyphs: Vec<(ab_glyph::GlyphId, f32)> = Vec::new();

        for c in text.chars() {
            let gid = font.glyph_id(c);

            if let Some(p) = prev {
                x += scaled.kerning(p, gid);
            }

            glyphs.push((gid, x));
            x += scaled.h_advance(gid);
            prev = Some(gid);
        }

        let width = x.ceil() as i32 + 2;
        let ascent = scaled.ascent();
        let height = (ascent - scaled.descent()).ceil() as i32 + 2;

        if width <= 0 || height <= 0 {
            return None;
        }

        let mut img = RgbaImage::from_pixel(
            width as u32,
            height as u32,
            image::Rgba([0, 0, 0, 0]),
        );

        // --------------------------------------------------------
        // Rasterize each glyph
        // --------------------------------------------------------
        for (gid, gx) in glyphs {
            let glyph = Glyph {
                id: gid,
                scale,
                position: point(gx, ascent),
            };

            let Some(outline) = font.outline_glyph(glyph) else {
                continue;
            };

            let bounds = outline.px_bounds();
            let ox = bounds.min.x as i32;
            let oy = bounds.min.y as i32;

            outline.draw(|px, py, cov| {
                if cov == 0 {
                    return;
                }

                let (r, g, b, a) = rgba;
                let alpha = (cov as u32 * a as u32 / 255) as u8;

                let X = ox + px as i32;
                let Y = oy + py as i32;

                if X < 0 || Y < 0 || X >= width || Y >= height {
                    return;
                }

                let p = img.get_pixel_mut(X as u32, Y as u32);

                if alpha > p[3] {
                    *p = image::Rgba([r, g, b, alpha]);
                }
            });
        }

        Some(img)
    }
}

// ============================================================================
// GPU-UPLOADABLE TEXT TEXTURE
// ============================================================================

#[derive(Clone, Debug)]
pub struct TextTexture {
    pub buffer: MemoryRenderBuffer,
    pub size: Size<i32, Logical>,
}

impl TextTexture {
    pub fn from_rgba(rgba: RgbaImage) -> Option<Self> {
        let w = rgba.width() as i32;
        let h = rgba.height() as i32;

        if w <= 0 || h <= 0 {
            return None;
        }

        let size = Size::<i32, Logical>::new(w, h);

        let buffer = MemoryRenderBuffer::from_slice(
            rgba.as_raw(),
            Fourcc::Abgr8888,
            size,
            1,
            Transform::Normal,
            Some(vec![Rectangle::from_size(size)]),
        );

        Some(Self { buffer, size })
    }

    pub fn element(
        &self,
        renderer: &mut GlesRenderer,
        pos: (i32, i32),
    ) -> Result<MemoryRenderBufferRenderElement<GlesRenderer>, GlesError> {
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (pos.0 as f64, pos.1 as f64),
            &self.buffer,
            Some(1.0),
            None,
            None,
            smithay::backend::renderer::element::Kind::Unspecified,
        )
    }
}
