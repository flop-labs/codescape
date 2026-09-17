//! The `CodeScape` widget: renders the repository as a 3D landscape of
//! directory blocks and text tiles, and handles fly-through navigation.
//!
//! Level of detail: every file is always drawn as one quad sampling the
//! minimap atlas. Files that are large on screen additionally get real SDF
//! glyph quads, nearest first, up to `GLYPH_BUDGET`.

use crate::atlas::{self, glyph_index, Image, FONT_COLS};
use crate::camera::{v3, Camera, View, V3};
use crate::layout::{Layout, Rect as GRect, COL_CHARS, FILE_LABEL_H, LINE_H};
use crate::scan::{self, SourceFile, CLASS_COUNT};
use crate::tour::Tour;
use makepad_widgets::makepad_draw::geometry::GeometryCube3D;
use makepad_widgets::*;
use std::path::PathBuf;
use std::time::Instant;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    pub DrawBox = {{DrawBox}} {
        uniform vp0: vec4(1.0, 0.0, 0.0, 0.0)
        uniform vp1: vec4(0.0, 1.0, 0.0, 0.0)
        uniform vp2: vec4(0.0, 0.0, 1.0, 0.0)
        uniform vp3: vec4(0.0, 0.0, 0.0, 1.0)
        uniform focal_px: 1000.0

        fn clip(self, p: vec3) -> vec4 {
            return self.vp0 * p.x + self.vp1 * p.y + self.vp2 * p.z + self.vp3;
        }
        varying local: vec3
        varying face: float
        varying cw: float

        fn vertex(self) -> vec4 {
            self.local = self.geom_pos + vec3(0.5, 0.5, 0.5);
            let p = self.box_pos + self.local * self.box_size;
            self.face = self.geom_id;
            let c = self.clip(p);
            self.cw = c.w;
            return vec4(c.x, c.y, (0.55 + 0.45 * c.z / c.w) * c.w, c.w);
        }

        fn fragment(self) -> vec4 {
            return self.pixel();
        }

        fn pixel(self) -> vec4 {
            let wpp = self.cw / self.focal_px;
            if self.face > 1.5 && self.face < 2.5 {
                let ex = min(self.local.x, 1.0 - self.local.x) * self.box_size.x;
                let ez = min(self.local.z, 1.0 - self.local.z) * self.box_size.z;
                let e = min(ex, ez);
                let rim = max(self.rim.w, wpp * 1.5);
                let k = 1.0 - smoothstep(rim * 0.5, rim, e);
                return vec4(mix(self.top.rgb, self.rim.rgb, k), 1.0);
            }
            let side = 0.5;
            if self.face < 1.5 {
                side = 0.62;
            }
            let shade = side * mix(0.45, 1.0, self.local.y);
            let lip = 1.0 - smoothstep(0.0, max(wpp * 1.5, 0.3), (1.0 - self.local.y) * self.box_size.y);
            return vec4(mix(self.top.rgb * shade, self.rim.rgb * 0.8, lip), 1.0);
        }
    }

    pub DrawTile = {{DrawTile}} {
        uniform vp0: vec4(1.0, 0.0, 0.0, 0.0)
        uniform vp1: vec4(0.0, 1.0, 0.0, 0.0)
        uniform vp2: vec4(0.0, 0.0, 1.0, 0.0)
        uniform vp3: vec4(0.0, 0.0, 0.0, 1.0)
        uniform focal_px: 1000.0

        fn clip(self, p: vec3) -> vec4 {
            return self.vp0 * p.x + self.vp1 * p.y + self.vp2 * p.z + self.vp3;
        }
        texture minimap: texture2d
        varying tex: vec2
        varying lp: vec2
        varying cw: float

        fn vertex(self) -> vec4 {
            let p = vec3(
                self.tile.x + self.geom_pos.x * self.tile.z,
                self.elev,
                self.tile.y + self.geom_pos.y * self.tile.w
            );
            self.tex = mix(self.uv.xy, self.uv.zw, self.geom_pos);
            self.lp = self.geom_pos * self.tile.zw;
            let c = self.clip(p);
            self.cw = c.w;
            return vec4(c.x, c.y, (0.55 + 0.45 * c.z / c.w - 0.00002) * c.w, c.w);
        }

        fn fragment(self) -> vec4 {
            return self.pixel();
        }

        fn pixel(self) -> vec4 {
            // Line height and world units per pixel, exact per fragment.
            let px = 2.0 * self.focal_px / self.cw;
            let wpp = self.cw / self.focal_px;
            let s = sample2d(self.minimap, self.tex);
            let fade = mix(1.0, 1.0 - smoothstep(3.0, 7.0, px), self.state.y);
            let bg = self.bg.rgb + vec3(0.05, 0.06, 0.08) * self.state.x;
            let col = bg * (1.0 - s.a * fade) + s.rgb * fade;
            let e = min(min(self.lp.x, self.tile.z - self.lp.x), min(self.lp.y, self.tile.w - self.lp.y));
            let bw = max(0.5, wpp * 1.2);
            // Rims on tiles that are only a few pixels wide would drown the text.
            let tile_px = min(self.tile.z, self.tile.w) / wpp;
            let rim_a = mix(0.06, self.rim.w, smoothstep(10.0, 80.0, tile_px));
            let k = (1.0 - smoothstep(bw * 0.5, bw, e)) * mix(rim_a, 1.0, self.state.x);
            return vec4(mix(col, self.rim.rgb, k), 1.0);
        }
    }

    pub DrawGlyph = {{DrawGlyph}} {
        uniform vp0: vec4(1.0, 0.0, 0.0, 0.0)
        uniform vp1: vec4(0.0, 1.0, 0.0, 0.0)
        uniform vp2: vec4(0.0, 0.0, 1.0, 0.0)
        uniform vp3: vec4(0.0, 0.0, 0.0, 1.0)
        uniform focal_px: 1000.0

        fn clip(self, p: vec3) -> vec4 {
            return self.vp0 * p.x + self.vp1 * p.y + self.vp2 * p.z + self.vp3;
        }
        texture font: texture2d
        varying tex: vec2
        varying cw: float

        fn vertex(self) -> vec4 {
            let p = vec3(
                self.gpos.x + self.geom_pos.x * self.gsize.x,
                self.gpos.y,
                self.gpos.z + self.geom_pos.y * self.gsize.y
            );
            self.tex = (self.cell + self.geom_pos) / vec2(16.0, 6.0);
            let c = self.clip(p);
            self.cw = c.w;
            return vec4(c.x, c.y, (0.55 + 0.45 * c.z / c.w - 0.00005) * c.w, c.w);
        }

        fn fragment(self) -> vec4 {
            return self.pixel();
        }

        fn pixel(self) -> vec4 {
            let px = self.gsize.y * self.focal_px / self.cw;
            let d = sample2d(self.font, self.tex).x;
            // SDF spread: 4 of the 64 texels per cell map onto 0.5 of range.
            let w = clamp(64.0 / max(px, 0.001) * 0.125 * 0.5, 0.02, 0.5);
            let a = smoothstep(0.5 - w, 0.5 + w, d) * smoothstep(self.fade.x, self.fade.y, px);
            return vec4(self.color.rgb * a, a);
        }
    }

    pub CodeScape = {{CodeScape}} {
        width: Fill, height: Fill
        font_file: dep("crate://makepad_widgets/resources/LiberationMono-Regular.ttf")
        draw_panel: { color: #0A1128D8 }
        draw_title: {
            color: #00B4D8
            text_style: <THEME_FONT_BOLD> { font_size: 15.0 }
        }
        draw_text: {
            color: #F5F7FA
            text_style: <THEME_FONT_CODE> { font_size: 10.0 }
        }
        draw_dim: {
            color: #A1A7AE
            text_style: <THEME_FONT_CODE> { font_size: 9.0 }
        }
        draw_caption: {
            color: #F5F7FA
            text_style: <THEME_FONT_BOLD> { font_size: 20.0 }
        }
    }
}

macro_rules! draw_shader_impl {
    ($ty:ident) => {
        impl LiveHook for $ty {
            fn before_apply(
                &mut self,
                cx: &mut Cx,
                apply: &mut Apply,
                index: usize,
                nodes: &[LiveNode],
            ) {
                self.draw_vars
                    .before_apply_init_shader(cx, apply, index, nodes, &self.geometry);
            }
            fn after_apply(
                &mut self,
                cx: &mut Cx,
                apply: &mut Apply,
                index: usize,
                nodes: &[LiveNode],
            ) {
                self.draw_vars
                    .after_apply_update_self(cx, apply, index, nodes, &self.geometry);
            }
        }

        #[allow(dead_code)]
        impl $ty {
            pub fn begin(&mut self, cx: &mut Cx2d, view: &View) {
                let m = &view.view_proj;
                self.draw_vars.set_uniform(cx, id!(vp0), &m[0..4]);
                self.draw_vars.set_uniform(cx, id!(vp1), &m[4..8]);
                self.draw_vars.set_uniform(cx, id!(vp2), &m[8..12]);
                self.draw_vars.set_uniform(cx, id!(vp3), &m[12..16]);
                self.draw_vars
                    .set_uniform(cx, id!(focal_px), &[view.focal_px]);
                self.many_instances = cx.begin_many_instances(&self.draw_vars);
            }
            pub fn draw(&mut self) {
                if let Some(mi) = &mut self.many_instances {
                    mi.instances.extend_from_slice(self.draw_vars.as_slice());
                }
            }
            /// Appends one instance given as raw slots, in field order.
            pub fn push(&mut self, slots: &[f32]) {
                if let Some(mi) = &mut self.many_instances {
                    mi.instances.extend_from_slice(slots);
                }
            }
            pub fn end(&mut self, cx: &mut Cx2d) {
                if let Some(mi) = self.many_instances.take() {
                    let area = cx.end_many_instances(mi);
                    self.draw_vars.area = cx.update_area_refs(self.draw_vars.area, area);
                }
            }
        }
    };
}

#[derive(Live, LiveRegister)]
#[repr(C)]
pub struct DrawBox {
    #[rust]
    pub many_instances: Option<ManyInstances>,
    #[live]
    pub geometry: GeometryCube3D,
    #[deref]
    pub draw_vars: DrawVars,
    #[live]
    pub box_pos: Vec3,
    #[live]
    pub box_size: Vec3,
    #[live]
    pub top: Vec4,
    #[live]
    pub rim: Vec4,
}
draw_shader_impl!(DrawBox);

#[derive(Live, LiveRegister)]
#[repr(C)]
pub struct DrawTile {
    #[rust]
    pub many_instances: Option<ManyInstances>,
    #[live]
    pub geometry: GeometryQuad2D,
    #[deref]
    pub draw_vars: DrawVars,
    #[live]
    pub tile: Vec4,
    #[live]
    pub elev: f32,
    #[live]
    pub uv: Vec4,
    #[live]
    pub bg: Vec4,
    #[live]
    pub rim: Vec4,
    /// x: hovered, y: glyph layer active.
    #[live]
    pub state: Vec2,
}
draw_shader_impl!(DrawTile);

#[derive(Live, LiveRegister)]
#[repr(C)]
pub struct DrawGlyph {
    #[rust]
    pub many_instances: Option<ManyInstances>,
    #[live]
    pub geometry: GeometryQuad2D,
    #[deref]
    pub draw_vars: DrawVars,
    #[live]
    pub gpos: Vec3,
    #[live]
    pub gsize: Vec2,
    #[live]
    pub cell: Vec2,
    /// Glyph height in pixels over which the glyph fades in.
    #[live]
    pub fade: Vec2,
    #[live]
    pub color: Vec4,
}
draw_shader_impl!(DrawGlyph);
const GLYPH_SLOTS: usize = 13;

const GLYPH_BUDGET: usize = 450_000;
/// Minimum on-screen line height (px) before a file gets real glyphs.
const GLYPH_MIN_PX: f32 = 3.0;

fn hex(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}

fn rgba(c: [f32; 3], a: f32) -> Vec4 {
    vec4(c[0], c[1], c[2], a)
}

/// Syntax colours, indexed by `scan::C_*`. FLOP palette (DESIGN.md) with
/// text-safe tints on the Base navy.
fn syntax_palette() -> [[f32; 3]; CLASS_COUNT] {
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
const ACCENTS: [u32; 4] = [0x00B4D8, 0x32D74B, 0x3D8BFF, 0xA1A7AE];

pub struct Scene {
    pub sources: Vec<SourceFile>,
    pub layout: Layout,
    pub repo: String,
    pub revision: String,
    glyph_on: Vec<bool>,
    glyph_count: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Pan,
    Orbit,
}

#[derive(Live, LiveHook, Widget)]
pub struct CodeScape {
    #[walk]
    walk: Walk,
    #[redraw]
    #[rust]
    area: Area,
    #[live]
    draw_box: DrawBox,
    #[live]
    draw_tile: DrawTile,
    #[live]
    draw_glyph: DrawGlyph,
    #[live]
    draw_panel: DrawColor,
    #[live]
    draw_title: DrawText,
    #[live]
    draw_text: DrawText,
    #[live]
    draw_dim: DrawText,
    #[live]
    draw_caption: DrawText,
    #[live]
    font_file: LiveDependency,

    #[rust]
    scene: Option<Scene>,
    #[rust(Camera { target: v3(0.0, 0.0, 0.0), dist: 1000.0, yaw: 0.0, pitch: 0.9 })]
    cam: Camera,
    #[rust]
    rect: Rect2,
    #[rust]
    next_frame: NextFrame,
    #[rust]
    last_time: f64,
    #[rust]
    fps: f64,
    #[rust]
    drag: Option<(Drag, DVec2)>,
    #[rust]
    mouse: DVec2,
    #[rust]
    hover: Option<usize>,
    #[rust]
    keys: Vec<KeyCode>,
    /// (from, to, start time once the first frame arrives, duration)
    #[rust]
    fly: Option<(Camera, Camera, Option<f64>, f64)>,
    #[rust]
    tour: Option<Tour>,
    #[rust]
    opts: Options,
    #[rust]
    frame_index: usize,
    #[rust]
    glyph_buf: Vec<f32>,
}

#[derive(Default, Clone, Copy)]
struct Rect2 {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Default)]
struct Options {
    parsed: bool,
    repo: Option<PathBuf>,
    tour: bool,
    tour_loop: bool,
    exit_after_tour: bool,
    record: Option<PathBuf>,
    record_fps: f64,
    atlas_size: usize,
    /// Still mode: jump to this tour time, save a screenshot and exit.
    at: Option<f64>,
    shot: Option<PathBuf>,
}

impl Options {
    fn parse() -> Options {
        let mut o = Options {
            parsed: true,
            record_fps: 30.0,
            atlas_size: 8192,
            ..Default::default()
        };
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--tour" => o.tour = true,
                "--loop" => o.tour_loop = true,
                "--exit-after-tour" => o.exit_after_tour = true,
                "--record" => o.record = args.next().map(PathBuf::from),
                "--record-fps" => {
                    o.record_fps = args.next().and_then(|v| v.parse().ok()).unwrap_or(30.0)
                }
                "--at" => o.at = args.next().and_then(|v| v.parse().ok()),
                "--shot" => o.shot = args.next().map(PathBuf::from),
                "--atlas" => {
                    o.atlas_size = args.next().and_then(|v| v.parse().ok()).unwrap_or(8192)
                }
                s if !s.starts_with('-') => o.repo = Some(PathBuf::from(s)),
                _ => {}
            }
        }
        if o.at.is_some() {
            o.tour = true;
        }
        if o.record.is_some() {
            o.tour = true;
            o.exit_after_tour = true;
        }
        o
    }
}

fn git(root: &std::path::Path, args: &[&str]) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn upload(cx: &mut Cx, img: Image, mips: bool) -> Texture {
    let format = if mips {
        TextureFormat::VecMipBGRAu8_32 {
            width: img.width,
            height: img.height,
            data: Some(img.data),
            max_level: Some(4),
            updated: TextureUpdated::Full,
        }
    } else {
        TextureFormat::VecBGRAu8_32 {
            width: img.width,
            height: img.height,
            data: Some(img.data),
            updated: TextureUpdated::Full,
        }
    };
    Texture::new_with_format(cx, format)
}

impl CodeScape {
    fn load(&mut self, cx: &mut Cx) {
        let t0 = Instant::now();
        let root = self.opts.repo.clone().unwrap_or_else(|| {
            PathBuf::from(git(
                &std::env::current_dir().unwrap(),
                &["rev-parse", "--show-toplevel"],
            ))
        });
        // Name from the origin remote, so worktrees still show the repo name.
        let origin = git(&root, &["remote", "get-url", "origin"]);
        let from_origin = origin
            .trim_end_matches('/')
            .rsplit(['/', ':'])
            .next()
            .map(|n| n.trim_end_matches(".git"));
        let repo = match from_origin {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => std::fs::canonicalize(&root)
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "repo".into()),
        };
        let revision = git(&root, &["rev-parse", "--short", "HEAD"]);
        let sources = scan::scan_repo(&root);
        let mut layout = Layout::build(&sources, &repo);
        let palette = syntax_palette();
        let (minimap, texel) =
            atlas::build_minimap(&mut layout, &sources, &palette, self.opts.atlas_size);
        let ttf = cx
            .get_dependency(self.font_file.as_str())
            .expect("bundled monospace font");
        let font = atlas::build_font(&ttf);
        let minimap = upload(cx, minimap, true);
        let font = upload(cx, font, false);
        self.draw_tile.draw_vars.set_texture(0, &minimap);
        self.draw_glyph.draw_vars.set_texture(0, &font);
        let root_rect = layout.dirs[0].rect;
        self.cam = overview(&layout);
        let n = layout.files.len();
        log!(
            "codescape: {} files, {} lines, {} dirs, world {:.0}x{:.0}, minimap texel {} in {:.2}s",
            n,
            layout.total_lines,
            layout.dirs.len(),
            root_rect.w,
            root_rect.h,
            texel,
            t0.elapsed().as_secs_f64()
        );
        if self.opts.tour {
            let paths: Vec<&str> = sources.iter().map(|s| s.path.as_str()).collect();
            let mut tour = Tour::flop_core(&layout, &paths, self.opts.tour_loop);
            if let Some(at) = self.opts.at {
                if let Some(c) = tour.advance(at) {
                    self.cam = c;
                }
            }
            self.tour = Some(tour);
        }
        self.scene = Some(Scene {
            sources,
            layout,
            repo,
            revision,
            glyph_on: vec![false; n],
            glyph_count: 0,
        });
    }

    fn view(&self) -> View {
        self.cam
            .view(self.rect.w.max(1.0) as f32, self.rect.h.max(1.0) as f32)
    }

    fn local(&self, abs: DVec2) -> (f32, f32) {
        ((abs.x - self.rect.x) as f32, (abs.y - self.rect.y) as f32)
    }

    /// Topmost file tile under a screen point.
    fn pick(&self, abs: DVec2) -> Option<usize> {
        let scene = self.scene.as_ref()?;
        let view = self.view();
        let (px, py) = self.local(abs);
        let d = view.ray(px, py);
        let mut best: Option<(f32, usize)> = None;
        for (i, f) in scene.layout.files.iter().enumerate() {
            if d.y.abs() < 1e-6 {
                break;
            }
            let t = (f.y - view.eye.y) / d.y;
            if t <= 0.0 || best.is_some_and(|b| b.0 <= t) {
                continue;
            }
            let p = view.eye.add(d.scale(t));
            if f.tile.contains(p.x, p.z) {
                best = Some((t, i));
            }
        }
        best.map(|b| b.1)
    }

    /// Ground point under the cursor, using the plane of whatever is there.
    fn cursor_ground(&self, abs: DVec2) -> Option<V3> {
        let view = self.view();
        let (px, py) = self.local(abs);
        let y = self
            .pick(abs)
            .and_then(|f| self.scene.as_ref().map(|s| s.layout.files[f].y))
            .unwrap_or(self.cam.target.y);
        view.hit_plane(px, py, y)
    }

    fn fly_to(&mut self, cx: &mut Cx, to: Camera, secs: f64) {
        self.fly = Some((self.cam, to, None, secs));
        self.tour = None;
        self.request_frame(cx);
    }

    fn request_frame(&mut self, cx: &mut Cx) {
        self.next_frame = cx.new_next_frame();
    }

    fn animating(&self) -> bool {
        self.fly.is_some()
            || self.tour.is_some()
            || !self.keys.is_empty()
            || self.opts.record.is_some()
    }

    fn tick(&mut self, cx: &mut Cx, time: f64) {
        let dt = if self.last_time == 0.0 {
            1.0 / 60.0
        } else {
            (time - self.last_time).min(0.1)
        };
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt) * 0.1;
        }
        self.last_time = time;

        if let Some(shot) = self.opts.shot.clone() {
            self.frame_index += 1;
            if self.frame_index == 8 {
                let _ = std::process::Command::new("xwd")
                    .args(["-name", "FLOP codescape", "-silent", "-out"])
                    .arg(shot)
                    .status();
                cx.quit();
            }
            self.area.redraw(cx);
            self.request_frame(cx);
            return;
        }
        if let Some(dir) = self.opts.record.clone() {
            // Capture the frame presented before this tick, then advance the
            // tour by a fixed step so the video is smooth at any render speed.
            if self.frame_index > 2 {
                let path = dir.join(format!("f{:05}.xwd", self.frame_index - 3));
                let _ = std::process::Command::new("xwd")
                    .args(["-name", "FLOP codescape", "-silent", "-out"])
                    .arg(path)
                    .status();
            }
            self.frame_index += 1;
        }
        let step = if self.opts.record.is_some() {
            1.0 / self.opts.record_fps
        } else {
            dt
        };
        if self.opts.record.is_some() && self.frame_index <= 3 {
            // Let the first frames settle before the tour clock starts.
        } else if let Some(tour) = &mut self.tour {
            match tour.advance(step) {
                Some(c) => self.cam = c,
                None => {
                    self.tour = None;
                    if self.opts.exit_after_tour {
                        cx.quit();
                    }
                }
            }
        }
        if let Some((from, to, start, secs)) = self.fly {
            let start = start.unwrap_or(time);
            self.fly = Some((from, to, Some(start), secs));
            let t = ((time - start) / secs).clamp(0.0, 1.0) as f32;
            let e = t * t * (3.0 - 2.0 * t);
            self.cam = from.lerp(&to, e);
            if t >= 1.0 {
                self.fly = None;
            }
        }
        if !self.keys.is_empty() {
            let upp = self.cam.units_per_px(self.rect.h as f32) * 600.0 * dt as f32;
            for k in self.keys.clone() {
                match k {
                    KeyCode::KeyW | KeyCode::ArrowUp => {
                        self.cam.target = self.cam.target.add(self.cam.ground_forward().scale(upp))
                    }
                    KeyCode::KeyS | KeyCode::ArrowDown => {
                        self.cam.target = self.cam.target.sub(self.cam.ground_forward().scale(upp))
                    }
                    KeyCode::KeyA | KeyCode::ArrowLeft => {
                        self.cam.target = self.cam.target.sub(self.cam.ground_right().scale(upp))
                    }
                    KeyCode::KeyD | KeyCode::ArrowRight => {
                        self.cam.target = self.cam.target.add(self.cam.ground_right().scale(upp))
                    }
                    KeyCode::KeyQ => self.cam.yaw += 1.2 * dt as f32,
                    KeyCode::KeyE => self.cam.yaw -= 1.2 * dt as f32,
                    KeyCode::KeyR => self.cam.pitch = (self.cam.pitch + 0.8 * dt as f32).min(1.55),
                    KeyCode::KeyF => self.cam.pitch = (self.cam.pitch - 0.8 * dt as f32).max(0.15),
                    KeyCode::KeyZ => self.cam.dist *= 1.0 - 1.5 * dt as f32,
                    KeyCode::KeyX => self.cam.dist *= 1.0 + 1.5 * dt as f32,
                    _ => {}
                }
            }
        }
        self.area.redraw(cx);
        if self.animating() {
            self.request_frame(cx);
        }
    }

    fn draw_scene(&mut self, cx: &mut Cx2d, view: &View) {
        let Some(scene) = self.scene.as_mut() else {
            return;
        };

        // Glyphs are collected first: tiles need to know which files have
        // them, but must be drawn before them.
        let mut glyphs = std::mem::take(&mut self.glyph_buf);
        glyphs.clear();
        collect_glyphs(scene, view, &mut glyphs);
        let scene = self.scene.as_ref().unwrap();
        let layout = &scene.layout;
        let accent = |d: usize| hex(ACCENTS[layout.dirs[d].top_level % ACCENTS.len()]);

        // Directory blocks.
        self.draw_box.begin(cx, view);
        for (i, d) in layout.dirs.iter().enumerate() {
            let r = d.rect;
            let lift = (d.depth as f32 * 0.018).min(0.09);
            self.draw_box.box_pos = vec3(r.x, d.y_base, r.z);
            self.draw_box.box_size = vec3(r.w, d.y_top - d.y_base, r.h);
            self.draw_box.top = vec4(0.063 + lift, 0.086 + lift, 0.16 + lift * 1.3, 1.0);
            let a = if i == 0 { hex(0x232A3E) } else { accent(i) };
            self.draw_box.rim = rgba(a, (d.label_h * 0.08).max(0.4));
            self.draw_box.draw();
        }
        self.draw_box.end(cx);

        // File tiles.
        self.draw_tile.begin(cx, view);
        for (i, f) in layout.files.iter().enumerate() {
            let t = f.tile;
            self.draw_tile.tile = vec4(t.x, t.z, t.w, t.h);
            self.draw_tile.elev = f.y;
            self.draw_tile.uv = vec4(f.uv[0], f.uv[1], f.uv[2], f.uv[3]);
            self.draw_tile.bg = vec4(0.035, 0.05, 0.1, 1.0);
            self.draw_tile.rim = rgba(accent(f.dir), 0.35);
            let hovered = if self.hover == Some(i) { 1.0 } else { 0.0 };
            self.draw_tile.state = vec2(hovered, if scene.glyph_on[i] { 1.0 } else { 0.0 });
            self.draw_tile.draw();
        }
        self.draw_tile.end(cx);

        self.draw_glyph.begin(cx, view);
        debug_assert_eq!(self.draw_glyph.draw_vars.as_slice().len(), GLYPH_SLOTS);
        self.draw_glyph.push(&glyphs);
        self.draw_glyph.end(cx);
        self.glyph_buf = glyphs;
    }

    fn draw_hud(&mut self, cx: &mut Cx2d) {
        let Some(scene) = self.scene.as_ref() else {
            self.draw_text.draw_abs(
                cx,
                dvec2(self.rect.x + 20.0, self.rect.y + 20.0),
                "codescape: scanning repository...",
            );
            return;
        };
        let (x, y, w, h) = (self.rect.x, self.rect.y, self.rect.w, self.rect.h);
        let l = &scene.layout;
        self.draw_panel.draw_abs(
            cx,
            Rect {
                pos: dvec2(x + 16.0, y + 16.0),
                size: dvec2(360.0, 84.0),
            },
        );
        self.draw_title
            .draw_abs(cx, dvec2(x + 30.0, y + 26.0), "FLOP  codescape");
        let stats = format!(
            "{} @ {}\n{} files  {:.2}M lines  {} dirs",
            scene.repo,
            scene.revision,
            l.files.len(),
            l.total_lines as f64 / 1e6,
            l.dirs.len()
        );
        self.draw_dim
            .draw_abs(cx, dvec2(x + 30.0, y + 56.0), &stats);

        // Recording throttles frames, so its fps figure would mislead.
        let perf = if self.opts.record.is_some() {
            format!(
                "{:>7} glyphs   {} tiles   GPU instanced",
                scene.glyph_count,
                l.files.len()
            )
        } else {
            format!(
                "{:>3.0} fps   {:>7} glyphs   {} tiles",
                self.fps,
                scene.glyph_count,
                l.files.len()
            )
        };
        self.draw_panel.draw_abs(
            cx,
            Rect {
                pos: dvec2(x + w - 356.0, y + 16.0),
                size: dvec2(340.0, 34.0),
            },
        );
        self.draw_dim
            .draw_abs(cx, dvec2(x + w - 342.0, y + 27.0), &perf);

        if let Some(hf) = self.hover {
            let f = &l.files[hf];
            let path = &scene.sources[f.file].path;
            let info = format!("{}   {} lines", path, f.lines);
            let pw = 24.0 + info.len() as f64 * 7.3;
            self.draw_panel.draw_abs(
                cx,
                Rect {
                    pos: dvec2(x + 16.0, y + h - 88.0),
                    size: dvec2(pw, 34.0),
                },
            );
            self.draw_text
                .draw_abs(cx, dvec2(x + 28.0, y + h - 79.0), &info);
        }
        if let Some(caption) = self.tour.as_ref().and_then(|t| t.caption()) {
            let cw = 40.0 + caption.len() as f64 * 12.5;
            self.draw_panel.draw_abs(
                cx,
                Rect {
                    pos: dvec2(x + (w - cw) * 0.5, y + h - 112.0),
                    size: dvec2(cw, 50.0),
                },
            );
            self.draw_caption.draw_abs(
                cx,
                dvec2(x + (w - cw) * 0.5 + 20.0, y + h - 101.0),
                caption,
            );
        }
        if self.opts.record.is_none() {
            let help = "drag pan  |  right-drag orbit  |  wheel zoom  |  click file: fly in  |  WASD QE RF  |  T tour  H home";
            self.draw_panel.draw_abs(
                cx,
                Rect {
                    pos: dvec2(x + 16.0, y + h - 40.0),
                    size: dvec2(help.len() as f64 * 6.6 + 24.0, 30.0),
                },
            );
            self.draw_dim
                .draw_abs(cx, dvec2(x + 28.0, y + h - 31.0), help);
        }
    }
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
fn collect_glyphs(scene: &mut Scene, view: &View, out: &mut Vec<f32>) {
    let layout = &scene.layout;
    let palette = syntax_palette();
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
                let col = palette[class[c] as usize];
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

impl Widget for CodeScape {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, _scope: &mut Scope) {
        if !self.opts.parsed {
            self.opts = Options::parse();
        }
        if let Some(ne) = self.next_frame.is_event(event) {
            self.tick(cx, ne.time);
        }
        match event.hits(cx, self.area) {
            Hit::FingerDown(fe) => {
                cx.set_key_focus(self.area);
                let orbit = fe.device.mouse_button().is_some_and(|b| !b.is_primary())
                    || fe.modifiers.shift
                    || fe.modifiers.alt;
                self.drag = Some((if orbit { Drag::Orbit } else { Drag::Pan }, fe.abs));
                self.tour = None;
                self.fly = None;
            }
            Hit::FingerMove(fe) => {
                if let Some((mode, last)) = self.drag {
                    let d = fe.abs - last;
                    match mode {
                        Drag::Pan => {
                            let upp = self.cam.units_per_px(self.rect.h as f32);
                            let fwd = upp / self.cam.pitch.sin().max(0.35);
                            self.cam.target = self
                                .cam
                                .target
                                .sub(self.cam.ground_right().scale(d.x as f32 * upp))
                                .add(self.cam.ground_forward().scale(d.y as f32 * fwd));
                        }
                        Drag::Orbit => {
                            self.cam.yaw -= d.x as f32 * 0.005;
                            self.cam.pitch =
                                (self.cam.pitch + d.y as f32 * 0.004).clamp(0.12, 1.55);
                        }
                    }
                    self.drag = Some((mode, fe.abs));
                    self.area.redraw(cx);
                }
            }
            Hit::FingerUp(fe) => {
                self.drag = None;
                if fe.is_over && (fe.abs - fe.abs_start).length() < 4.0 {
                    if let (Some(f), Some(scene)) = (self.pick(fe.abs), self.scene.as_ref()) {
                        let to = reading_view(&scene.layout, f, self.rect.h as f32);
                        self.fly_to(cx, to, 1.2);
                    }
                }
            }
            Hit::FingerHoverOver(fh) | Hit::FingerHoverIn(fh) => {
                self.mouse = fh.abs;
                let h = self.pick(fh.abs);
                if h != self.hover {
                    self.hover = h;
                    self.area.redraw(cx);
                }
            }
            Hit::FingerScroll(fs) => {
                let amount = if fs.scroll.y != 0.0 {
                    fs.scroll.y
                } else {
                    fs.scroll.x
                } as f32;
                // Wheels step per notch (Makepad's X11 speed curve is erratic);
                // trackpads zoom continuously.
                let f = if fs.device.is_mouse() {
                    if amount < 0.0 {
                        0.84
                    } else {
                        1.0 / 0.84
                    }
                } else {
                    (amount.clamp(-200.0, 200.0) * 0.002).exp()
                };
                let anchor = self.cursor_ground(fs.abs);
                self.cam.dist = (self.cam.dist * f).clamp(2.0, 1e6);
                if let Some(p) = anchor {
                    self.cam.target = p.lerp(self.cam.target, f);
                }
                self.tour = None;
                self.fly = None;
                self.area.redraw(cx);
            }
            Hit::KeyDown(ke) => match ke.key_code {
                KeyCode::KeyT => {
                    if let Some(scene) = self.scene.as_ref() {
                        self.fly = None;
                        let paths: Vec<&str> =
                            scene.sources.iter().map(|s| s.path.as_str()).collect();
                        let mut t = Tour::flop_core(&scene.layout, &paths, true);
                        t.start_from(self.cam);
                        self.tour = Some(t);
                        self.request_frame(cx);
                    }
                }
                KeyCode::KeyH | KeyCode::Home => {
                    if let Some(scene) = self.scene.as_ref() {
                        let to = overview(&scene.layout);
                        self.fly_to(cx, to, 1.2);
                    }
                }
                KeyCode::Escape => {
                    self.tour = None;
                    self.fly = None;
                }
                k => {
                    if !ke.is_repeat && !self.keys.contains(&k) {
                        self.keys.push(k);
                        self.tour = None;
                        self.request_frame(cx);
                    }
                }
            },
            Hit::KeyUp(ke) => self.keys.retain(|k| *k != ke.key_code),
            _ => {}
        }
    }

    fn draw_walk(&mut self, cx: &mut Cx2d, _scope: &mut Scope, walk: Walk) -> DrawStep {
        if !self.opts.parsed {
            self.opts = Options::parse();
        }
        let r = cx.walk_turtle_with_area(&mut self.area, walk);
        self.rect = Rect2 {
            x: r.pos.x,
            y: r.pos.y,
            w: r.size.x,
            h: r.size.y,
        };
        if self.scene.is_none() {
            self.load(cx);
            self.last_time = 0.0;
            self.request_frame(cx);
        }
        let view = self.view();
        self.draw_scene(cx, &view);
        self.draw_hud(cx);
        DrawStep::done()
    }
}
