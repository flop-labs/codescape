//! World layout. The ground is the XZ plane (y up). One world unit is one
//! character wide; a text line is `LINE_H` units tall. Each file becomes a
//! flat tile of text columns, and each directory a raised block holding its
//! children, shelf-packed recursively.

use crate::scan::SourceFile;

pub const LINE_H: f32 = 2.0;
pub const COL_CHARS: usize = 100;
pub const COL_GUTTER: f32 = 8.0;
/// Glyph cell height of a file's name label, in world units.
pub const FILE_LABEL_H: f32 = 7.0;
const FILE_LABEL_GAP: f32 = 3.0;
const ITEM_GAP: f32 = 10.0;

#[derive(Clone, Copy, Default, Debug)]
pub struct Rect {
    pub x: f32,
    pub z: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, z: f32) -> bool {
        x >= self.x && z >= self.z && x <= self.x + self.w && z <= self.z + self.h
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.z + self.h * 0.5)
    }
}

pub struct FileNode {
    pub file: usize,
    pub dir: usize,
    pub name: String,
    pub lines: usize,
    /// Text area of the tile (label sits above it).
    pub tile: Rect,
    pub y: f32,
    pub lines_per_col: usize,
    /// Minimap texture rectangle, normalised.
    pub uv: [f32; 4],
}

impl FileNode {
    /// World position (x, z) of the start of `line`.
    pub fn char_pos(&self, line: usize) -> (f32, f32) {
        let col = line / self.lines_per_col;
        let row = line % self.lines_per_col;
        (
            self.tile.x + col as f32 * (COL_CHARS as f32 + COL_GUTTER),
            self.tile.z + row as f32 * LINE_H,
        )
    }
}

pub struct DirNode {
    pub name: String,
    pub path: String,
    pub depth: usize,
    pub rect: Rect,
    pub y_base: f32,
    pub y_top: f32,
    pub label_h: f32,
    /// Size rank of the top-level directory this one belongs to.
    pub top_level: usize,
    pub dirs: Vec<usize>,
    pub files: Vec<usize>,
    pub lines: usize,
}

pub struct Layout {
    pub dirs: Vec<DirNode>,
    pub files: Vec<FileNode>,
    pub total_lines: usize,
}

impl Layout {
    pub fn build(sources: &[SourceFile], repo_name: &str) -> Layout {
        let mut l = Layout {
            dirs: Vec::new(),
            files: Vec::new(),
            total_lines: 0,
        };
        l.dirs
            .push(DirNode::new(repo_name.to_string(), String::new(), 0));
        for (i, src) in sources.iter().enumerate() {
            let mut dir = 0;
            let parts: Vec<&str> = src.path.split('/').collect();
            for (d, part) in parts[..parts.len() - 1].iter().enumerate() {
                dir = match l.dirs[dir].dirs.iter().find(|c| l.dirs[**c].name == *part) {
                    Some(c) => *c,
                    None => {
                        let id = l.dirs.len();
                        let path = parts[..=d].join("/");
                        l.dirs.push(DirNode::new(part.to_string(), path, d + 1));
                        l.dirs[dir].dirs.push(id);
                        id
                    }
                };
            }
            let lines = src.line_count();
            // Square-ish tiles: k columns of L/k lines each.
            let col_w = COL_CHARS as f32 + COL_GUTTER;
            let cols = ((lines as f32 * LINE_H / col_w).sqrt().round() as usize).max(1);
            let lines_per_col = lines.div_ceil(cols);
            let w = cols as f32 * COL_CHARS as f32 + (cols - 1) as f32 * COL_GUTTER;
            let h = lines_per_col as f32 * LINE_H;
            let fid = l.files.len();
            l.files.push(FileNode {
                file: i,
                dir,
                name: parts[parts.len() - 1].to_string(),
                lines,
                tile: Rect {
                    x: 0.0,
                    z: 0.0,
                    w,
                    h,
                },
                y: 0.0,
                lines_per_col,
                uv: [0.0; 4],
            });
            l.dirs[dir].files.push(fid);
            l.total_lines += lines;
        }
        l.measure(0);
        // Accent slots go to the largest top-level directories first.
        let mut top = l.dirs[0].dirs.clone();
        top.sort_by_key(|d| std::cmp::Reverse(l.dirs[*d].lines));
        l.place(0, 0.0, 0.0, 0.0);
        for (rank, d) in top.into_iter().enumerate() {
            l.set_accent(d, rank);
        }
        l
    }

    /// Computes each directory's size (bottom-up); positions are relative.
    fn measure(&mut self, d: usize) -> (f32, f32) {
        let children = self.dirs[d].dirs.clone();
        let mut items: Vec<(bool, usize, f32, f32)> = Vec::new();
        let mut lines = 0;
        for c in children {
            let (w, h) = self.measure(c);
            lines += self.dirs[c].lines;
            items.push((true, c, w, h));
        }
        for f in self.dirs[d].files.clone() {
            let t = self.files[f].tile;
            lines += self.files[f].lines;
            let name_w = FILE_LABEL_H * 0.5 * self.files[f].name.len() as f32;
            items.push((
                false,
                f,
                t.w.max(name_w),
                t.h + FILE_LABEL_H + FILE_LABEL_GAP,
            ));
        }
        self.dirs[d].lines = lines;
        items.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap().then(a.1.cmp(&b.1)));
        let area: f32 = items
            .iter()
            .map(|i| (i.2 + ITEM_GAP) * (i.3 + ITEM_GAP))
            .sum();
        let max_w = items.iter().map(|i| i.2).fold(0.0, f32::max);
        let target = (max_w + ITEM_GAP).max(area.sqrt() * 1.1);
        let pad = (area.sqrt() * 0.02).clamp(6.0, 80.0);
        let label_scale = if self.dirs[d].depth == 1 { 0.08 } else { 0.05 };
        let label_h = (area.sqrt() * label_scale).clamp(9.0, 1000.0);
        let top = pad + label_h * 1.4;
        let mut sky = Skyline::new(target);
        let (mut extent_w, mut extent_h) = (0.0f32, 0.0f32);
        for it in &items {
            let (x, y) = sky.place(it.2 + ITEM_GAP, it.3 + ITEM_GAP);
            let rel = Rect {
                x: pad + x,
                z: top + y,
                w: it.2,
                h: it.3,
            };
            if it.0 {
                self.dirs[it.1].rect = rel;
            } else {
                let t = &mut self.files[it.1].tile;
                t.x = rel.x;
                t.z = rel.z + FILE_LABEL_H + FILE_LABEL_GAP;
            }
            extent_w = extent_w.max(x + it.2);
            extent_h = extent_h.max(y + it.3);
        }
        let name_w = label_h * 0.5 * (self.dirs[d].name.len() as f32 + 2.0);
        let w = extent_w.max(name_w) + pad * 2.0;
        let h = top + extent_h + pad;
        self.dirs[d].label_h = label_h;
        self.dirs[d].rect.w = w;
        self.dirs[d].rect.h = h;
        (w, h)
    }

    /// Converts relative positions to world positions (top-down).
    fn place(&mut self, d: usize, ox: f32, oz: f32, y_base: f32) {
        let r = self.dirs[d].rect;
        let (x, z) = (ox + r.x, oz + r.z);
        let step = if d == 0 {
            20.0
        } else {
            ((r.w * r.h).sqrt() * 0.025).clamp(3.0, 400.0)
        };
        let y_top = y_base + step;
        let dn = &mut self.dirs[d];
        dn.rect.x = x;
        dn.rect.z = z;
        dn.y_base = y_base;
        dn.y_top = y_top;
        for f in dn.files.clone() {
            let fnode = &mut self.files[f];
            fnode.tile.x += x;
            fnode.tile.z += z;
            fnode.y = y_top;
        }
        for c in self.dirs[d].dirs.clone() {
            self.place(c, x, z, y_top);
        }
    }

    fn set_accent(&mut self, d: usize, rank: usize) {
        self.dirs[d].top_level = rank;
        for c in self.dirs[d].dirs.clone() {
            self.set_accent(c, rank);
        }
    }

    pub fn find_dir(&self, path: &str) -> Option<usize> {
        self.dirs.iter().position(|d| d.path == path)
    }
}

impl DirNode {
    fn new(name: String, path: String, depth: usize) -> Self {
        DirNode {
            name,
            path,
            depth,
            rect: Rect::default(),
            y_base: 0.0,
            y_top: 0.0,
            label_h: 0.0,
            top_level: 0,
            dirs: Vec::new(),
            files: Vec::new(),
            lines: 0,
        }
    }
}

/// Bottom-left skyline packer over a fixed-width strip.
struct Skyline {
    width: f32,
    /// (x, y, w) segments, left to right, covering the full width.
    segs: Vec<(f32, f32, f32)>,
}

impl Skyline {
    fn new(width: f32) -> Self {
        Skyline {
            width,
            segs: vec![(0.0, 0.0, width)],
        }
    }

    /// Places a `w`×`h` box at the lowest available spot (then leftmost).
    fn place(&mut self, w: f32, h: f32) -> (f32, f32) {
        let w = w.min(self.width);
        let mut best: Option<(f32, f32)> = None;
        for i in 0..self.segs.len() {
            let x = self.segs[i].0;
            if x + w > self.width + 0.01 {
                break;
            }
            // Resting height: the highest segment under [x, x + w).
            let mut y = 0.0f32;
            for s in &self.segs[i..] {
                if s.0 >= x + w - 0.01 {
                    break;
                }
                y = y.max(s.1);
            }
            if best.is_none_or(|b| y < b.1 - 0.01) {
                best = Some((x, y));
            }
        }
        let (x, y) = best.unwrap_or((0.0, 0.0));
        self.raise(x, w, y + h);
        (x, y)
    }

    fn raise(&mut self, x: f32, w: f32, top: f32) {
        let mut out = Vec::with_capacity(self.segs.len() + 2);
        for &(sx, sy, sw) in &self.segs {
            let (a, b) = (sx, sx + sw);
            if b <= x || a >= x + w {
                out.push((sx, sy, sw));
                continue;
            }
            if a < x {
                out.push((a, sy, x - a));
            }
            if b > x + w {
                out.push((x + w, sy, b - (x + w)));
            }
        }
        out.push((x, top, w));
        out.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap());
        // Merge equal-height neighbours to keep the list short.
        let mut merged: Vec<(f32, f32, f32)> = Vec::with_capacity(out.len());
        for s in out {
            match merged.last_mut() {
                Some(l) if (l.1 - s.1).abs() < 0.01 => l.2 += s.2,
                _ => merged.push(s),
            }
        }
        self.segs = merged;
    }
}
