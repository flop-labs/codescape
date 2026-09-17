//! CPU-built textures: the minimap atlas (every file's text, downsampled) and
//! a signed-distance-field atlas of monospace glyphs.

use crate::layout::{Layout, COL_CHARS, LINE_H};
use crate::scan::{SourceFile, GLYPH_OTHER};

pub struct Image {
    pub width: usize,
    pub height: usize,
    /// 0xAARRGGBB per texel, premultiplied alpha.
    pub data: Vec<u32>,
}

fn pack(c: [f32; 3], a: f32) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    (b(a) << 24) | (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2])
}

/// Packs every file's minimap into one atlas no larger than `max_size`²,
/// choosing the finest texel size (in world units) that fits. Writes each
/// file's normalised uv rectangle into the layout.
pub fn build_minimap(
    layout: &mut Layout,
    sources: &[SourceFile],
    palette: &[[f32; 3]],
    max_size: usize,
) -> (Image, f32) {
    let mut order: Vec<usize> = (0..layout.files.len()).collect();
    order.sort_by(|a, b| {
        layout.files[*b]
            .tile
            .h
            .partial_cmp(&layout.files[*a].tile.h)
            .unwrap()
    });
    let mut placed = Vec::new();
    let mut texel = 1.0f32;
    let mut height = 0;
    for t in [1.0f32, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0] {
        placed.clear();
        let (mut x, mut y, mut row_h) = (0usize, 0usize, 0usize);
        for &f in &order {
            let r = layout.files[f].tile;
            let w = (r.w / t).ceil() as usize + 2;
            let h = (r.h / t).ceil() as usize + 2;
            if x + w > max_size {
                x = 0;
                y += row_h;
                row_h = 0;
            }
            placed.push((f, x, y, w, h));
            x += w;
            row_h = row_h.max(h);
        }
        height = y + row_h;
        texel = t;
        if height <= max_size {
            break;
        }
    }
    let width = max_size;
    let height = height.next_multiple_of(64);
    let mut data = vec![0u32; width * height];

    // Rasterise files in parallel; each thread writes to its own buffers.
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let chunk = placed.len().div_ceil(threads).max(1);
    // (file, atlas x, atlas y, width, texels) per rasterised file.
    type Rastered = (usize, usize, usize, usize, Vec<u32>);
    let results: Vec<Vec<Rastered>> = std::thread::scope(|s| {
        let handles: Vec<_> = placed
            .chunks(chunk)
            .map(|part| {
                let layout = &*layout;
                s.spawn(move || {
                    part.iter()
                        .map(|&(f, x, y, w, h)| {
                            let img = raster_file(layout, sources, palette, f, w, h, texel);
                            (f, x, y, w, img)
                        })
                        .collect()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (f, x, y, w, img) in results.into_iter().flatten() {
        let h = img.len() / w;
        for row in 0..h {
            let dst = (y + row) * width + x;
            data[dst..dst + w].copy_from_slice(&img[row * w..(row + 1) * w]);
        }
        let r = layout.files[f].tile;
        layout.files[f].uv = [
            (x as f32 + 1.0) / width as f32,
            (y as f32 + 1.0) / height as f32,
            (x as f32 + 1.0 + r.w / texel) / width as f32,
            (y as f32 + 1.0 + r.h / texel) / height as f32,
        ];
    }
    (
        Image {
            width,
            height,
            data,
        },
        texel,
    )
}

fn raster_file(
    layout: &Layout,
    sources: &[SourceFile],
    palette: &[[f32; 3]],
    f: usize,
    w: usize,
    h: usize,
    texel: f32,
) -> Vec<u32> {
    let node = &layout.files[f];
    let src = &sources[node.file];
    let mut acc = vec![[0f32; 4]; w * h];
    for i in 0..src.line_count() {
        let (x0, z0) = node.char_pos(i);
        let (x0, z0) = (x0 - node.tile.x, z0 - node.tile.z);
        let ty = 1 + ((z0 + LINE_H * 0.5) / texel) as usize;
        let (text, class) = src.line(i);
        for (c, (&ch, &k)) in text.iter().zip(class).enumerate().take(COL_CHARS) {
            if ch == b' ' {
                continue;
            }
            let tx = 1 + ((x0 + c as f32 + 0.5) / texel) as usize;
            let a = &mut acc[ty.min(h - 1) * w + tx.min(w - 1)];
            let p = palette[k as usize];
            a[0] += p[0];
            a[1] += p[1];
            a[2] += p[2];
            a[3] += 1.0;
        }
    }
    // Chars that fit in one texel; code is ~40% ink, so boost coverage.
    let capacity = (texel * texel / LINE_H).max(0.5);
    acc.iter()
        .map(|a| {
            if a[3] == 0.0 {
                0
            } else {
                // Premultiplied, so mipmaps average correctly.
                let n = a[3];
                let cov = (n / capacity * 2.0).min(1.0);
                pack([a[0] / n * cov, a[1] / n * cov, a[2] / n * cov], cov)
            }
        })
        .collect()
}

pub const FONT_COLS: usize = 16;
pub const FONT_ROWS: usize = 6;
pub const FONT_CELL_W: usize = 32;
pub const FONT_CELL_H: usize = 64;
/// SDF spread, in low-res texels, mapped onto [0, 1].
pub const FONT_SPREAD: f32 = 4.0;

pub fn glyph_index(ch: u8) -> usize {
    if ch == GLYPH_OTHER || !(32..=126).contains(&ch) {
        95
    } else {
        (ch - 32) as usize
    }
}

/// Builds a 16×6 grid SDF atlas for ASCII 32..126 plus a middle dot, from a
/// monospace TTF whose advance is 0.6 em. One cell is one character wide and
/// one text line tall.
pub fn build_font(ttf: &[u8]) -> Image {
    const HS: usize = 6;
    let font =
        fontdue::Font::from_bytes(ttf, fontdue::FontSettings::default()).expect("font parse");
    let (cw, ch) = (FONT_CELL_W * HS, FONT_CELL_H * HS);
    let px = cw as f32 / 0.6;
    let lm = font.horizontal_line_metrics(px).expect("line metrics");
    let baseline = (ch as f32 - (lm.ascent - lm.descent)) * 0.5 + lm.ascent;
    let width = FONT_COLS * FONT_CELL_W;
    let height = FONT_ROWS * FONT_CELL_H;
    let mut data = vec![0u32; width * height];
    let chars: Vec<char> = (32u8..=126)
        .map(|c| c as char)
        .chain(['\u{00b7}'])
        .collect();
    let spread_hi = FONT_SPREAD * HS as f32;
    for (gi, c) in chars.iter().enumerate() {
        let (m, bmp) = font.rasterize(*c, px);
        let mut inside = vec![false; cw * ch];
        let left = m.xmin;
        let top = baseline as i32 - (m.ymin + m.height as i32);
        for yy in 0..m.height {
            for xx in 0..m.width {
                let (x, y) = (left + xx as i32, top + yy as i32);
                if x >= 0
                    && y >= 0
                    && (x as usize) < cw
                    && (y as usize) < ch
                    && bmp[yy * m.width + xx] >= 128
                {
                    inside[y as usize * cw + x as usize] = true;
                }
            }
        }
        let d_out = edt(&inside, cw, ch, true);
        let d_in = edt(&inside, cw, ch, false);
        let (gx, gy) = (gi % FONT_COLS, gi / FONT_COLS);
        for ly in 0..FONT_CELL_H {
            for lx in 0..FONT_CELL_W {
                let i = (ly * HS + HS / 2) * cw + lx * HS + HS / 2;
                let sd = if inside[i] {
                    d_in[i].sqrt()
                } else {
                    -d_out[i].sqrt()
                } as f32;
                let v = 0.5 + sd / (2.0 * spread_hi);
                data[(gy * FONT_CELL_H + ly) * width + gx * FONT_CELL_W + lx] =
                    pack([v, v, v], 1.0);
            }
        }
    }
    Image {
        width,
        height,
        data,
    }
}

/// Squared Euclidean distance to the nearest pixel where `mask == target`
/// (Felzenszwalb & Huttenlocher lower envelope of parabolas).
fn edt(mask: &[bool], w: usize, h: usize, target: bool) -> Vec<f64> {
    const INF: f64 = 1e20;
    let mut grid: Vec<f64> = mask
        .iter()
        .map(|m| if *m == target { 0.0 } else { INF })
        .collect();
    let n = w.max(h);
    let (mut f, mut d, mut v, mut z) = (
        vec![0f64; n],
        vec![0f64; n],
        vec![0usize; n],
        vec![0f64; n + 1],
    );
    for x in 0..w {
        for y in 0..h {
            f[y] = grid[y * w + x];
        }
        edt_1d(&f[..h], &mut d[..h], &mut v, &mut z);
        for y in 0..h {
            grid[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&grid[y * w..(y + 1) * w]);
        edt_1d(&f[..w], &mut d[..w], &mut v, &mut z);
        grid[y * w..(y + 1) * w].copy_from_slice(&d[..w]);
    }
    grid
}

#[allow(clippy::needless_range_loop)] // index form mirrors the paper
fn edt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let qf = q as f64;
        let mut s;
        loop {
            let pf = v[k] as f64;
            s = ((f[q] + qf * qf) - (f[v[k]] + pf * pf)) / (2.0 * qf - 2.0 * pf);
            if s <= z[k] {
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let dq = q as f64 - v[k] as f64;
        d[q] = dq * dq + f[v[k]];
    }
}
