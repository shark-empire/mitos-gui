//! Stage 6: procedural tray icon rasterization.
//!
//! Small CPU-rasterized glyphs uploaded through the same
//! MemoryRenderBuffer pipeline as text and wallpaper.

use image::RgbaImage;

type Rgba = (u8, u8, u8, u8);

fn put(img: &mut RgbaImage, x: i32, y: i32, c: Rgba) {
    if x < 0 || y < 0 || x as u32 >= img.width() || y as u32 >= img.height() {
        return;
    }

    let p = img.get_pixel_mut(x as u32, y as u32);

    if c.3 > p[3] {
        *p = image::Rgba([c.0, c.1, c.2, c.3]);
    }
}

/// Wi-Fi arcs + dot. `arcs` = 0..3 signal strength.
pub fn wifi_icon(size: i32, arcs: u8, color: Rgba) -> RgbaImage {
    let mut img = RgbaImage::new(size as u32, size as u32);

    let cx = size as f32 / 2.0;
    let cy = (size - 3) as f32;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - cx;
            let dy = cy - y as f32; // positive = up

            if dy < 0.0 {
                continue;
            }

            let d = (dx * dx + dy * dy).sqrt();

            if d <= 1.8 {
                put(&mut img, x, y, color);
                continue;
            }

            if dx.abs() > dy {
                continue; // 90° cone
            }

            for i in 0..arcs {
                let r = 4.0 + i as f32 * 4.0;

                if (d - r).abs() <= 1.1 {
                    put(&mut img, x, y, color);
                }
            }
        }
    }

    img
}

/// Wired-connection glyph (monitor + stand).
pub fn ethernet_icon(size: i32, color: Rgba) -> RgbaImage {
    let mut img = RgbaImage::new(size as u32, size as u32);

    let (x0, x1) = (3, size - 4);
    let (y0, y1) = (3, size - 7);
    let cx = size / 2;

    for x in x0..=x1 {
        put(&mut img, x, y0, color);
        put(&mut img, x, y1, color);
    }

    for y in y0..=y1 {
        put(&mut img, x0, y, color);
        put(&mut img, x1, y, color);
    }

    for y in y1..=y1 + 2 {
        put(&mut img, cx, y, color);
    }

    for x in (cx - 2)..=(cx + 2) {
        put(&mut img, x, y1 + 2, color);
    }

    img
}

/// Speaker + waves, or an X when muted.
pub fn volume_icon(size: i32, level: u8, muted: bool, color: Rgba) -> RgbaImage {
    let mut img = RgbaImage::new(size as u32, size as u32);

    let cy = size / 2;

    // Speaker body
    for y in (cy - 2)..=(cy + 2) {
        for x in 2..=4 {
            put(&mut img, x, y, color);
        }
    }

    // Cone
    for x in 5..=8 {
        let half = 2 + (x - 5);

        for y in (cy - half)..=(cy + half) {
            put(&mut img, x, y, color);
        }
    }

    if muted {
        for t in 0..5 {
            put(&mut img, 11 + t, cy - 2 + t, color);
            put(&mut img, 15 - t, cy - 2 + t, color);
        }
    } else {
        // Waves (right half only)
        for y in 0..size {
            for x in 10..size {
                let dx = x as f32 - 9.0;
                let dy = y as f32 - cy as f32;

                if dx <= 0.0 || dy.abs() > dx {
                    continue;
                }

                let d = (dx * dx + dy * dy).sqrt();

                if level > 0 && (d - 4.0).abs() <= 1.0 {
                    put(&mut img, x, y, color);
                }

                if level > 50 && (d - 7.0).abs() <= 1.0 {
                    put(&mut img, x, y, color);
                }
            }
        }
    }

    img
}

/// Battery outline + proportional fill.
pub fn battery_icon(
    capacity: u8,
    charging: bool,
    color: Rgba,
) -> RgbaImage {
    let (w, h) = (24, 12);
    let mut img = RgbaImage::new(w, h);

    for x in 0..=19 {
        put(&mut img, x, 0, color);
        put(&mut img, x, 11, color);
    }

    for y in 0..=11 {
        put(&mut img, 0, y, color);
        put(&mut img, 19, y, color);
    }

    for x in 21..=22 {
        for y in 4..=7 {
            put(&mut img, x, y, color);
        }
    }

    let fill: Rgba = if charging {
        (120, 220, 140, 255)
    } else if capacity <= 20 {
        (235, 90, 90, 255)
    } else {
        color
    };

    let fw = (16 * capacity as i32) / 100;

    for x in 2..(2 + fw) {
        for y in 2..=9 {
            put(&mut img, x, y, fill);
        }
    }

    img
}
