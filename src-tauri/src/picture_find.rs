//! Finding the game picture inside another program's window (a streaming or upscaling app shows the
//! game with its own panels and buttons around it): the part of the window that moves.

use crate::capture::FrameData;

/// Blocks of this many pixels each way are compared between frames.
const BLOCK: u32 = 8;

/// The part of the window (x, y, w, h fractions) that changed between the frames — the game picture —
/// without the plain black bars at its sides; None when too little of the window moved (a paused
/// game, a still menu) to tell. All frames must be the same size.
pub fn moving_part(frames: &[&FrameData]) -> Option<[f64; 4]> {
    let first = frames.first()?;
    let (w, h) = (first.w, first.h);
    if frames.len() < 2 || w < BLOCK * 8 || h < BLOCK * 8 || frames.iter().any(|f| f.w != w || f.h != h) {
        return None;
    }
    let (bw, bh) = ((w / BLOCK) as usize, (h / BLOCK) as usize);
    // a block moved: its average brightness changed between two frames in a row
    let mut moved = vec![false; bw * bh];
    for pair in frames.windows(2) {
        let (a, b) = (block_means(pair[0], bw, bh), block_means(pair[1], bw, bh));
        for i in 0..moved.len() {
            moved[i] |= (a[i] - b[i]).abs() > 4.0;
        }
    }
    // the biggest group of moving blocks (touching, gaps of a block allowed: a dark still patch inside
    // the game) is the picture; a counter or a clock in the other program's panels is a small one
    let grown = grow(&moved, bw, bh);
    let (gx0, gy0, gx1, gy1) = biggest_group(&grown, bw, bh)?;
    // (the moving blocks themselves, not the neighbours added to join them up)
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for by in gy0..=gy1 {
        for bx in gx0..=gx1 {
            if moved[by * bw + bx] {
                (x0, y0, x1, y1) = (x0.min(bx), y0.min(by), x1.max(bx), y1.max(by));
            }
        }
    }
    if x0 > x1 {
        return None;
    }
    let area = ((x1 - x0 + 1) * (y1 - y0 + 1)) as f64 / (bw * bh) as f64;
    if area < 0.08 {
        return None;
    }
    // in pixels, then without black bars (letterboxing) at its edges
    let last = frames[frames.len() - 1];
    let mut r = ((x0 as u32) * BLOCK, (y0 as u32) * BLOCK, ((x1 - x0 + 1) as u32) * BLOCK, ((y1 - y0 + 1) as u32) * BLOCK);
    r = trim_black(last, r);
    let (x, y, rw, rh) = r;
    Some([x as f64 / w as f64, y as f64 / h as f64, rw.min(w - x) as f64 / w as f64, rh.min(h - y) as f64 / h as f64])
}

fn luma(f: &FrameData, x: u32, y: u32) -> f64 {
    let i = ((y * f.w + x) * 4) as usize;
    (f.rgba[i] as f64 * 0.299) + (f.rgba[i + 1] as f64 * 0.587) + (f.rgba[i + 2] as f64 * 0.114)
}

/// The average brightness of each block (sampled every other pixel).
fn block_means(f: &FrameData, bw: usize, bh: usize) -> Vec<f64> {
    let mut out = vec![0.0; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let (mut sum, mut n) = (0.0, 0.0);
            for y in (0..BLOCK).step_by(2) {
                for x in (0..BLOCK).step_by(2) {
                    sum += luma(f, bx as u32 * BLOCK + x, by as u32 * BLOCK + y);
                    n += 1.0;
                }
            }
            out[by * bw + bx] = sum / n;
        }
    }
    out
}

/// Moving blocks with their neighbours (so a few still blocks inside the picture don't split it).
fn grow(m: &[bool], bw: usize, bh: usize) -> Vec<bool> {
    let mut out = vec![false; m.len()];
    for y in 0..bh {
        for x in 0..bw {
            if m[y * bw + x] {
                for yy in y.saturating_sub(1)..(y + 2).min(bh) {
                    for xx in x.saturating_sub(1)..(x + 2).min(bw) {
                        out[yy * bw + xx] = true;
                    }
                }
            }
        }
    }
    out
}

/// The bounding box (x0, y0, x1, y1 in blocks, inclusive) of the biggest group of touching blocks.
fn biggest_group(m: &[bool], bw: usize, bh: usize) -> Option<(usize, usize, usize, usize)> {
    let mut seen = vec![false; m.len()];
    let mut best: Option<(usize, (usize, usize, usize, usize))> = None;
    for start in 0..m.len() {
        if !m[start] || seen[start] {
            continue;
        }
        let (mut n, mut bb) = (0usize, (usize::MAX, usize::MAX, 0usize, 0usize));
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % bw, i / bw);
            n += 1;
            bb = (bb.0.min(x), bb.1.min(y), bb.2.max(x), bb.3.max(y));
            let mut push = |j: usize| {
                if m[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < bw {
                push(i + 1);
            }
            if y > 0 {
                push(i - bw);
            }
            if y + 1 < bh {
                push(i + bw);
            }
        }
        if best.is_none_or(|(bn, _)| n > bn) {
            best = Some((n, bb));
        }
    }
    best.map(|(_, bb)| bb)
}

/// The rectangle without the rows and columns at its edges that are plain black (letterboxing).
fn trim_black(f: &FrameData, (mut x, mut y, mut w, mut h): (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let (fw, fh) = (f.w, f.h);
    w = w.min(fw - x);
    h = h.min(fh - y);
    let black_col = |cx: u32, y: u32, h: u32| (y..y + h).step_by(2).all(|yy| luma(f, cx, yy) < 16.0);
    let black_row = |cy: u32, x: u32, w: u32| (x..x + w).step_by(2).all(|xx| luma(f, xx, cy) < 16.0);
    while w > 16 && black_col(x, y, h) {
        x += 1;
        w -= 1;
    }
    while w > 16 && black_col(x + w - 1, y, h) {
        w -= 1;
    }
    while h > 16 && black_row(y, x, w) {
        y += 1;
        h -= 1;
    }
    while h > 16 && black_row(y + h - 1, x, w) {
        h -= 1;
    }
    (x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 320 x 200 "window": grey panels, a counter that changes in the corner, and a "game" at
    /// (40, 20)-(240, 140) with black bars 10 px wide on its left and right whose middle changes.
    fn frame(t: u8) -> FrameData {
        let (w, h) = (320u32, 200u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let v: u8 = if (40..240).contains(&x) && (20..140).contains(&y) {
                    if x < 50 || x >= 230 { 0 } else { 60u8.wrapping_add(t.wrapping_mul(40)).wrapping_add((x / 16) as u8 * 7) }
                } else if (290..300).contains(&x) && (180..190).contains(&y) {
                    t.wrapping_mul(90) // the other program's frame counter
                } else {
                    120
                };
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        FrameData { w, h, rgba, seq: 0 }
    }

    #[test]
    fn finds_the_moving_game_without_its_black_bars() {
        let (a, b, c) = (frame(0), frame(1), frame(2));
        let r = moving_part(&[&a, &b, &c]).expect("found");
        let px = (r[0] * 320.0, r[1] * 200.0, r[2] * 320.0, r[3] * 200.0);
        assert!((px.0 - 50.0).abs() <= 8.0 && (px.1 - 20.0).abs() <= 8.0, "top-left {px:?}");
        assert!((px.0 + px.2 - 230.0).abs() <= 8.0 && (px.1 + px.3 - 140.0).abs() <= 8.0, "bottom-right {px:?}");
    }

    #[test]
    fn a_still_window_tells_nothing() {
        let (a, b) = (frame(0), frame(0));
        assert_eq!(moving_part(&[&a, &b]), None);
    }
}
