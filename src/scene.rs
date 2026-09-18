//! The scene: what the repository looks like as geometry, and which glyphs a
//! given camera needs. None of this touches the renderer, so it can be tested
//! and benchmarked without a GPU or a window.

use crate::atlas::{glyph_index, FONT_COLS};
use crate::camera::{v3, Camera, View, V3};
use crate::layout::{Layout, Rect as GRect, COL_CHARS, FILE_LABEL_H, LINE_H};
use crate::overlay::{mark_palette, Overlay, M_NONE};
use crate::scan::{SourceFile, CLASS_COUNT};

pub const GLYPH_SLOTS: usize = 13;

pub const GLYPH_BUDGET: usize = 450_000;
/// Minimum on-screen line height (px) before a file gets real glyphs.
pub const GLYPH_MIN_PX: f32 = 3.0;

pub fn hex(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}

/// Syntax colours, indexed by `scan::C_*`. FLOP palette (DESIGN.md) with
/// text-safe tints on the Base navy.
pub fn syntax_palette() -> [[f32; 3]; CLASS_COUNT] {
    [
        hex(0x000000), // space
        hex(0xC9D1DC), // ident
        hex(0x00B4D8), // keyword — FLOP Cyan
        hex(0x7FDBEE), // type
        hex(0x32D74B), // string — Electric Green
        hex(0x6FA8FF), // number — FLOP Blue, lifted for text
        hex(0x6B7785), // comment — Grey
        hex(0x8A93A0), // punct
        hex(0x3DC5E0), // heading / attribute
        hex(0xF5F7FA), // call — Ice White
    ]
}

/// Accent per top-level directory (DESIGN.md chart series).
pub const ACCENTS: [u32; 4] = [0x00B4D8, 0x32D74B, 0x3D8BFF, 0xA1A7AE];

pub struct Scene {
    pub sources: Vec<SourceFile>,
    pub layout: Layout,
    pub repo: String,
    pub revision: String,
    /// Diff or trace marks; empty when neither is active.
    pub overlay: Overlay,
    pub glyph_on: Vec<bool>,
    pub glyph_count: usize,
}

/// Instance slots for one glyph, in `DrawGlyph` field order.
#[rustfmt::skip]
fn glyph(pos: V3, size: (f32, f32), ch: u8, fade: (f32, f32), col: [f32; 3]) -> [f32; GLYPH_SLOTS] {
    let gi = glyph_index(ch);
    [
        pos.x, pos.y, pos.z, size.0, size.1,
        (gi % FONT_COLS) as f32, (gi / FONT_COLS) as f32,
        fade.0, fade.1, col[0], col[1], col[2], 1.0,
    ]
}

/// A flat label with glyph cells `h` tall starting at `pos`.
fn push_label(out: &mut Vec<f32>, text: &[u8], pos: V3, h: f32, col: [f32; 3], fade: (f32, f32)) {
    for (k, ch) in text.iter().enumerate() {
        if *ch != b' ' {
            let p = v3(pos.x + k as f32 * h * 0.5, pos.y, pos.z);
            out.extend_from_slice(&glyph(p, (h * 0.5, h), *ch, fade, col));
        }
    }
}

/// Emits glyph instances: directory and file labels, then the text of the
/// files that are largest on screen until `GLYPH_BUDGET` is reached.
pub fn collect_glyphs(scene: &mut Scene, view: &View, out: &mut Vec<f32>) {
    let layout = &scene.layout;
    let palette = syntax_palette();
    let marks = mark_palette();
    let accent = |d: usize| hex(ACCENTS[layout.dirs[d].top_level % ACCENTS.len()]);
    for (i, d) in layout.dirs.iter().enumerate() {
        let r = d.rect;
        let h = d.label_h;
        let label_rect = GRect {
            x: r.x,
            z: r.z,
            w: r.w,
            h: h * 1.5,
        };
        if line_px(view, &label_rect, d.y_top) * h / LINE_H < 1.5
            || !on_screen(view, &label_rect, d.y_top)
        {
            continue;
        }
        let pad = (r.w.min(r.h) * 0.02).clamp(6.0, 80.0);
        let col = if i == 0 { hex(0xF5F7FA) } else { accent(i) };
        let name = if i == 0 {
            format!("{} @ {}", d.name, scene.revision)
        } else {
            format!("{}/", d.name)
        };
        let pos = v3(r.x + pad, d.y_top, r.z + pad * 0.6 + h * 0.2);
        push_label(out, name.as_bytes(), pos, h, col, (1.5, 3.0));
    }
    for f in &layout.files {
        let lr = GRect {
            x: f.tile.x,
            z: f.tile.z - FILE_LABEL_H - 2.0,
            w: f.tile.w,
            h: FILE_LABEL_H,
        };
        if line_px(view, &lr, f.y) * FILE_LABEL_H / LINE_H < 2.0 || !on_screen(view, &lr, f.y) {
            continue;
        }
        let pos = v3(lr.x, f.y, lr.z);
        push_label(
            out,
            f.name.as_bytes(),
            pos,
            FILE_LABEL_H,
            hex(0xDDE3EA),
            (2.0, 4.0),
        );
    }

    let mut cand: Vec<(f32, usize)> = Vec::new();
    for (i, f) in layout.files.iter().enumerate() {
        scene.glyph_on[i] = false;
        let px = line_px(view, &f.tile, f.y);
        if px >= GLYPH_MIN_PX && on_screen(view, &f.tile, f.y) {
            cand.push((px, i));
        }
    }
    cand.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    for &(_, i) in &cand {
        if out.len() / GLYPH_SLOTS >= GLYPH_BUDGET {
            break;
        }
        scene.glyph_on[i] = true;
        let f = &layout.files[i];
        let src = &scene.sources[f.file];
        let region = visible_region(view, f.y);
        for line in 0..src.line_count() {
            let (x0, z0) = f.char_pos(line);
            if z0 + LINE_H < region.z
                || z0 > region.z + region.h
                || x0 + (COL_CHARS as f32) < region.x
                || x0 > region.x + region.w
            {
                continue;
            }
            let (text, class) = src.line(line);
            let mark = scene.overlay.mark(f.file, line);
            let c0 = (region.x - x0).floor().max(0.0) as usize;
            let c1 = ((region.x + region.w - x0).ceil().max(0.0) as usize)
                .min(COL_CHARS)
                .min(text.len());
            for c in c0..c1 {
                let ch = text[c];
                if ch == b' ' {
                    continue;
                }
                let pos = v3(x0 + c as f32, f.y, z0);
                let col = if mark == M_NONE {
                    palette[class[c] as usize]
                } else {
                    marks[mark as usize]
                };
                let fade = (GLYPH_MIN_PX, GLYPH_MIN_PX * 2.0);
                out.extend_from_slice(&glyph(pos, (1.0, LINE_H), ch, fade, col));
            }
        }
    }
    scene.glyph_count = out.len() / GLYPH_SLOTS;
}

/// On-screen height in pixels of one text line at the nearest point of `r`.
fn line_px(view: &View, r: &GRect, y: f32) -> f32 {
    let nx = view.eye.x.clamp(r.x, r.x + r.w);
    let nz = view.eye.z.clamp(r.z, r.z + r.h);
    let d = v3(nx, y, nz).sub(view.eye).len().max(1e-3);
    LINE_H * view.focal_px / d
}

/// Conservative test whether a ground rectangle intersects the viewport.
fn on_screen(view: &View, r: &GRect, y: f32) -> bool {
    let (w, h) = view.size;
    let corners = [
        (r.x, r.z),
        (r.x + r.w, r.z),
        (r.x, r.z + r.h),
        (r.x + r.w, r.z + r.h),
    ];
    let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (cx, cz) in corners {
        match view.project(v3(cx, y, cz)) {
            Some((sx, sy, _)) => {
                minx = minx.min(sx);
                maxx = maxx.max(sx);
                miny = miny.min(sy);
                maxy = maxy.max(sy);
            }
            // A corner behind the camera: only accept if the rect surrounds us.
            None => return r.contains(view.eye.x, view.eye.z) || line_px(view, r, y) > 1.0,
        }
    }
    maxx >= 0.0 && minx <= w && maxy >= 0.0 && miny <= h
}

/// Ground-plane rectangle (at height `y`) that can hold readable glyphs:
/// the view footprint, limited to where a line is at least `GLYPH_MIN_PX`.
fn visible_region(view: &View, y: f32) -> GRect {
    let reach = LINE_H * view.focal_px / GLYPH_MIN_PX;
    let dy = (view.eye.y - y).abs();
    let r = (reach * reach - dy * dy).max(0.0).sqrt();
    let (mut x0, mut z0, mut x1, mut z1) = (
        view.eye.x - r,
        view.eye.z - r,
        view.eye.x + r,
        view.eye.z + r,
    );
    let (w, h) = view.size;
    let mut fx0 = f32::MAX;
    let mut fz0 = f32::MAX;
    let mut fx1 = f32::MIN;
    let mut fz1 = f32::MIN;
    let mut all_hit = true;
    for (sx, sy) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        match view.hit_plane(sx, sy, y) {
            Some(p) => {
                fx0 = fx0.min(p.x);
                fz0 = fz0.min(p.z);
                fx1 = fx1.max(p.x);
                fz1 = fz1.max(p.z);
            }
            None => all_hit = false,
        }
    }
    if all_hit {
        x0 = x0.max(fx0);
        z0 = z0.max(fz0);
        x1 = x1.min(fx1);
        z1 = z1.min(fz1);
    }
    GRect {
        x: x0,
        z: z0,
        w: (x1 - x0).max(0.0),
        h: (z1 - z0).max(0.0),
    }
}

/// Whole-repo view.
pub fn overview(l: &Layout) -> Camera {
    let r = l.dirs[0].rect;
    let (cx, cz) = r.center();
    Camera {
        target: v3(cx, 0.0, cz + r.h * 0.04),
        dist: r.w.max(r.h) * 1.05,
        yaw: -0.12,
        pitch: 0.95,
    }
}

/// A camera from which the top of a file tile is readable.
pub fn reading_view(l: &Layout, f: usize, height_px: f32) -> Camera {
    let n = &l.files[f];
    let (_, cz) = n.tile.center();
    let tx = n.tile.x + (COL_CHARS as f32 * 0.5).min(n.tile.w * 0.5);
    let focal = height_px * 0.5 / (crate::camera::FOV_Y * 0.5).tan();
    let dist = LINE_H * focal / 16.0;
    let tz = (n.tile.z + dist * 0.35).min(cz);
    Camera {
        target: v3(tx, n.y, tz),
        dist,
        yaw: 0.0,
        pitch: 1.05,
    }
}
