//! Smooth: the FM-1's 240 x 240 picture, `SCALE` times bigger, smooth and crisp. A Lanczos-3 enlargement draws the
//! curves and diagonals of letters smoothly (and evens out the dot patterns some firmwares, like Erik37, dither their
//! screens with), then a light unsharp mask (a Gaussian blur of radius 2 taken away at 0.8) gives the edges back their
//! bite. The same as the iOS app's Core Image filters (CILanczosScaleTransform, CIUnsharpMask). From 1.3.9; before,
//! Scale2x twice and a 1-2-1 blur, which turned dithered letters into small stairs. The original picture is never
//! changed: this makes a copy.

use eframe::egui::Color32;

/// How many times bigger the smooth picture is.
pub const SCALE: usize = 4;
/// The unsharp mask: the blur's radius (its sigma, in output pixels) and how much of the difference is added back.
const SHARPEN_RADIUS: f32 = 2.0;
const SHARPEN_AMOUNT: f32 = 0.8;

fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-6 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let px = std::f32::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

/// For each output position along one side: the source indices and their weights (normalised, edges clamped).
fn taps(size: usize, scale: usize) -> Vec<Vec<(usize, f32)>> {
    (0..size * scale)
        .map(|o| {
            let c = (o as f32 + 0.5) / scale as f32 - 0.5;
            let first = c.floor() as isize - 2;
            let mut t: Vec<(usize, f32)> = (first..first + 6)
                .map(|i| (i.clamp(0, size as isize - 1) as usize, lanczos3(c - i as f32)))
                .collect();
            let sum: f32 = t.iter().map(|&(_, w)| w).sum();
            for p in &mut t {
                p.1 /= sum;
            }
            t
        })
        .collect()
}

/// Lanczos-3, `scale` times bigger: across, then down. Floats, three channels.
fn enlarge(src: &[Color32], size: usize, scale: usize) -> Vec<[f32; 3]> {
    let big = size * scale;
    let t = taps(size, scale);
    let mut across = vec![[0f32; 3]; size * big]; // size rows of big columns
    for y in 0..size {
        let row = &src[y * size..(y + 1) * size];
        for (x, tx) in t.iter().enumerate() {
            let mut v = [0f32; 3];
            for &(i, w) in tx {
                let c = row[i];
                v[0] += w * c.r() as f32;
                v[1] += w * c.g() as f32;
                v[2] += w * c.b() as f32;
            }
            across[y * big + x] = v;
        }
    }
    let mut out = vec![[0f32; 3]; big * big];
    for (y, ty) in t.iter().enumerate() {
        for x in 0..big {
            let mut v = [0f32; 3];
            for &(i, w) in ty {
                let a = across[i * big + x];
                v[0] += w * a[0];
                v[1] += w * a[1];
                v[2] += w * a[2];
            }
            out[y * big + x] = v;
        }
    }
    out
}

/// A Gaussian blur (sigma `radius`), across then down, edges clamped.
fn gaussian(src: &[[f32; 3]], size: usize, radius: f32) -> Vec<[f32; 3]> {
    let reach = (radius * 3.0).ceil() as isize;
    let mut k: Vec<f32> = (-reach..=reach).map(|i| (-(i * i) as f32 / (2.0 * radius * radius)).exp()).collect();
    let sum: f32 = k.iter().sum();
    for w in &mut k {
        *w /= sum;
    }
    let clamp = |i: isize| i.clamp(0, size as isize - 1) as usize;
    let mut across = vec![[0f32; 3]; size * size];
    for y in 0..size {
        for x in 0..size {
            let mut v = [0f32; 3];
            for (j, &w) in k.iter().enumerate() {
                let p = src[y * size + clamp(x as isize + j as isize - reach)];
                v[0] += w * p[0];
                v[1] += w * p[1];
                v[2] += w * p[2];
            }
            across[y * size + x] = v;
        }
    }
    let mut out = vec![[0f32; 3]; size * size];
    for y in 0..size {
        for x in 0..size {
            let mut v = [0f32; 3];
            for (j, &w) in k.iter().enumerate() {
                let p = across[clamp(y as isize + j as isize - reach) * size + x];
                v[0] += w * p[0];
                v[1] += w * p[1];
                v[2] += w * p[2];
            }
            out[y * size + x] = v;
        }
    }
    out
}

/// The picture, `SCALE` times bigger, smooth and crisp.
pub fn smooth(src: &[Color32], size: usize) -> Vec<Color32> {
    let big = size * SCALE;
    let enlarged = enlarge(src, size, SCALE);
    let blurred = gaussian(&enlarged, big, SHARPEN_RADIUS);
    enlarged
        .iter()
        .zip(&blurred)
        .map(|(e, b)| {
            let v = |k: usize| (e[k] + SHARPEN_AMOUNT * (e[k] - b[k])).round().clamp(0.0, 255.0) as u8;
            Color32::from_rgb(v(0), v(1), v(2))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Color32 = Color32::WHITE;
    const K: Color32 = Color32::BLACK;

    #[test]
    fn flat_stays_flat() {
        let out = smooth(&vec![Color32::from_rgb(10, 200, 30); 64], 8);
        assert_eq!(out.len(), 32 * 32);
        assert!(out.iter().all(|&c| c == Color32::from_rgb(10, 200, 30)));
    }

    #[test]
    fn an_edge_stays_sharp() {
        // Black left half, white right half: well inside each half it's still pure black and pure white, and the
        // step from one to the other is short (a few output pixels, not a long ramp).
        let mut src = vec![K; 64];
        for y in 0..8 {
            for x in 4..8 {
                src[y * 8 + x] = W;
            }
        }
        let out = smooth(&src, 8);
        let row: Vec<u8> = (0..32).map(|x| out[16 * 32 + x].r()).collect();
        assert_eq!(row[2], 0);
        assert_eq!(row[29], 255);
        let ramp = row.iter().filter(|&&v| v > 20 && v < 235).count();
        assert!(ramp <= 6, "edge ramp {ramp} pixels: {row:?}");
    }

    #[test]
    fn a_diagonal_is_smooth() {
        // A staircase, white above the diagonal: the enlarged edge has in-between shades, not 4x4 blocks.
        let mut src = vec![K; 64];
        for y in 0..8 {
            for x in y..8 {
                src[y * 8 + x] = W;
            }
        }
        let out = smooth(&src, 8);
        let mids = out.iter().filter(|c| c.r() > 30 && c.r() < 225).count();
        assert!(mids > 20, "{mids} in-between pixels");
    }
}

#[cfg(test)]
mod timing {
    #[test]
    #[ignore]
    fn full_screen_time() {
        let src: Vec<_> = (0..240 * 240).map(|i| eframe::egui::Color32::from_rgb((i % 251) as u8, (i % 17 * 13) as u8, 40)).collect();
        let t = std::time::Instant::now();
        for _ in 0..10 {
            std::hint::black_box(super::smooth(&src, 240));
        }
        println!("smooth 240 -> 960: {:?} each", t.elapsed() / 10);
    }
}
