//! The icon: a white microphone on a coloured disc, drawn in code so the
//! tray can show a different colour per state. Also used by build.rs to make
//! the .exe's icon, so it must not depend on anything outside std.
#![allow(dead_code)]

pub const APP: [u8; 3] = [0x39, 0x49, 0xAB];
pub const STOPPED: [u8; 3] = [0x8A, 0x8A, 0x8A];
pub const LISTENING: [u8; 3] = [0x2E, 0x7D, 0x32];
pub const HOLDING: [u8; 3] = [0xE6, 0x51, 0x00];
pub const ERROR: [u8; 3] = [0xC6, 0x28, 0x28];

fn seg_dist(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (px, py, dx, dy) = (x - ax, y - ay, bx - ax, by - ay);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (ex, ey) = (px - t * dx, py - t * dy);
    (ex * ex + ey * ey).sqrt()
}

/// Coordinates are 0..1 across the icon, y down.
fn mic(x: f32, y: f32) -> bool {
    let head = seg_dist(x, y, 0.5, 0.27, 0.5, 0.45) < 0.125;
    let (dx, dy) = (x - 0.5, y - 0.47);
    let arc = dy >= 0.0 && ((dx * dx + dy * dy).sqrt() - 0.215).abs() < 0.04;
    let stem = (x - 0.5).abs() < 0.04 && y > 0.68 && y < 0.80;
    let base = (x - 0.5).abs() < 0.15 && (y - 0.80).abs() < 0.04;
    head || arc || stem || base
}

fn disc(x: f32, y: f32) -> bool {
    let (dx, dy) = (x - 0.5, y - 0.5);
    dx * dx + dy * dy < 0.48 * 0.48
}

/// RGBA pixels, top row first.
pub fn render(size: u32, bg: [u8; 3]) -> Vec<u8> {
    const N: u32 = 4; // supersampling per axis
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut inside, mut white) = (0u32, 0u32);
            for sy in 0..N {
                for sx in 0..N {
                    let x = (px as f32 + (sx as f32 + 0.5) / N as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / N as f32) / size as f32;
                    if disc(x, y) {
                        inside += 1;
                        white += mic(x, y) as u32;
                    }
                }
            }
            let g = if inside > 0 { white as f32 / inside as f32 } else { 0.0 };
            for c in bg {
                out.push((c as f32 + (255.0 - c as f32) * g).round() as u8);
            }
            out.push((inside as f32 / (N * N) as f32 * 255.0).round() as u8);
        }
    }
    out
}

/// One icon image in the format .ico files and CreateIconFromResourceEx use:
/// a BITMAPINFOHEADER, BGRA rows bottom-up, then an (unused) AND mask.
pub fn bmp_entry(size: u32, bg: [u8; 3]) -> Vec<u8> {
    let rgba = render(size, bg);
    let mask_row = size.div_ceil(32) * 4;
    let mut v = Vec::with_capacity(40 + rgba.len() + (mask_row * size) as usize);
    v.extend(40u32.to_le_bytes());
    v.extend((size as i32).to_le_bytes());
    v.extend((size as i32 * 2).to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(32u16.to_le_bytes());
    v.extend(0u32.to_le_bytes());
    v.extend((size * size * 4).to_le_bytes());
    v.extend([0u8; 16]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            v.extend([rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    v.resize(v.len() + (mask_row * size) as usize, 0);
    v
}

/// A complete .ico file holding one image per size.
pub fn ico(sizes: &[u32], bg: [u8; 3]) -> Vec<u8> {
    let entries: Vec<Vec<u8>> = sizes.iter().map(|&s| bmp_entry(s, bg)).collect();
    let mut v = Vec::new();
    v.extend(0u16.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend((entries.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * entries.len() as u32;
    for (&s, e) in sizes.iter().zip(&entries) {
        let dim = (s % 256) as u8; // 0 means 256
        v.extend([dim, dim, 0, 0]);
        v.extend(1u16.to_le_bytes());
        v.extend(32u16.to_le_bytes());
        v.extend((e.len() as u32).to_le_bytes());
        v.extend(offset.to_le_bytes());
        offset += e.len() as u32;
    }
    for e in entries {
        v.extend(e);
    }
    v
}
