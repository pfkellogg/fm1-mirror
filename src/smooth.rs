//! Smooth: takes the jaggies off the FM-1's 240 x 240 picture. Scale2x twice follows diagonals and curves
//! (it only looks at which neighbours are the same colour, so flat areas and straight edges stay as they are),
//! then a light blur softens the small steps left. The original picture is never changed: this makes a copy.

use eframe::egui::Color32;

/// How many times bigger the smooth picture is.
pub const SCALE: usize = 4;

/// One Scale2x pass: a `size` x `size` picture to twice that.
fn scale2x(src: &[Color32], size: usize) -> Vec<Color32> {
    let out_size = size * 2;
    let mut out = vec![Color32::BLACK; out_size * out_size];
    for y in 0..size {
        for x in 0..size {
            let e = src[y * size + x];
            let b = src[y.saturating_sub(1) * size + x];
            let h = src[(y + 1).min(size - 1) * size + x];
            let d = src[y * size + x.saturating_sub(1)];
            let f = src[y * size + (x + 1).min(size - 1)];
            let (mut e0, mut e1, mut e2, mut e3) = (e, e, e, e);
            if b != h && d != f {
                if d == b {
                    e0 = d;
                }
                if b == f {
                    e1 = f;
                }
                if d == h {
                    e2 = d;
                }
                if h == f {
                    e3 = f;
                }
            }
            let o = y * 2 * out_size + x * 2;
            out[o] = e0;
            out[o + 1] = e1;
            out[o + out_size] = e2;
            out[o + out_size + 1] = e3;
        }
    }
    out
}

/// A 1-2-1 blur across and down.
fn soften(src: &[Color32], size: usize) -> Vec<Color32> {
    let mut across = vec![[0u16; 3]; size * size];
    for y in 0..size {
        for x in 0..size {
            let (l, m, r) = (src[y * size + x.saturating_sub(1)], src[y * size + x], src[y * size + (x + 1).min(size - 1)]);
            for k in 0..3 {
                across[y * size + x][k] = l[k] as u16 + 2 * m[k] as u16 + r[k] as u16;
            }
        }
    }
    let mut out = vec![Color32::BLACK; size * size];
    for y in 0..size {
        let (up, down) = (y.saturating_sub(1), (y + 1).min(size - 1));
        for x in 0..size {
            let (u, m, d) = (across[up * size + x], across[y * size + x], across[down * size + x]);
            let v = |k: usize| ((u[k] + 2 * m[k] + d[k] + 8) / 16) as u8;
            out[y * size + x] = Color32::from_rgb(v(0), v(1), v(2));
        }
    }
    out
}

/// The picture, `SCALE` times bigger and smooth.
pub fn smooth(src: &[Color32], size: usize) -> Vec<Color32> {
    let twice = scale2x(src, size);
    soften(&scale2x(&twice, size * 2), size * SCALE)
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
    fn a_diagonal_loses_its_steps() {
        // A staircase: white above the diagonal. Scale2x fills the corners of each step.
        let mut src = vec![K; 64];
        for y in 0..8 {
            for x in y..8 {
                src[y * 8 + x] = W;
            }
        }
        let out = scale2x(&src, 8);
        assert_eq!(out[(3 * 2 + 1) * 16 + 2 * 2 + 1], K); // the black side stays black
        assert_eq!(out[(3 * 2) * 16 + 3 * 2], W);
        assert_ne!(out[(3 * 2 + 1) * 16 + 3 * 2], src[3 * 8 + 3]); // the step's corner changed
    }
}
