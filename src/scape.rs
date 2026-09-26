//! The `CodeScape` widget: renders the repository as a 3D landscape of
//! directory blocks and text tiles, and handles fly-through navigation.
//!
//! Level of detail: every file is always drawn as one quad sampling the
//! minimap atlas. Files that are large on screen additionally get real SDF
//! glyph quads, nearest first, up to `GLYPH_BUDGET`.

use crate::atlas::{self, Image};
use crate::camera::{v3, Camera, View, V3};
use crate::capture::rgba_pixels;
use crate::diff;
use crate::layout::Layout;
use crate::overlay::{FileState, Overlay};
use crate::scan::{self, SourceFile};
use crate::scene::{
    collect_glyphs, hex, overview, reading_view, syntax_palette, Scene, ACCENTS, GLYPH_SLOTS,
};
use crate::tour::Tour;
use crate::trace::Trace;
use makepad_widgets::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

script_mod! {
    use mod.prelude.widgets.*
    use mod.widgets.*
    use mod.math.*
    use mod.shader.*
    use mod.draw
    use mod.geom

    mod.draw.DrawSceneTexture = mod.std.set_type_default() do #(DrawSceneTexture::script_shader(vm)){
        ..mod.draw.DrawQuad
        scene_texture: texture_2d(float)

        pixel: fn() {
            return self.scene_texture.sample_as_bgra(self.pos);
        }
    }

    mod.draw.DrawBox = mod.std.set_type_default() do #(DrawBox::script_shader(vm)){
        alpha_blend: false
        backface_culling: true
        vertex_pos: vertex_position(vec4f)
        fb0: fragment_output(0, vec4f)
        draw_call: uniform_buffer(draw.DrawCallUniforms)
        draw_pass: uniform_buffer(draw.DrawPassUniforms)
        draw_list: uniform_buffer(draw.DrawListUniforms)
        geom: vertex_buffer(geom.CubeVertex, geom.CubeGeom)

        vp0: uniform(vec4(1.0, 0.0, 0.0, 0.0))
        vp1: uniform(vec4(0.0, 1.0, 0.0, 0.0))
        vp2: uniform(vec4(0.0, 0.0, 1.0, 0.0))
        vp3: uniform(vec4(0.0, 0.0, 0.0, 1.0))
        focal_px: uniform(1000.0)

        local: varying(vec3f)
        face: varying(float)
        cw: varying(float)

        clip: fn(p: vec3) -> vec4 {
            return self.vp0 * p.x + self.vp1 * p.y + self.vp2 * p.z + self.vp3;
        }

        vertex: fn() {
            self.local = self.geom.geom_pos + vec3(0.5, 0.5, 0.5);
            let p = self.box_pos + self.local * self.box_size;
            self.face = self.geom.geom_id;
            let c = self.clip(p);
            self.cw = c.w;
            self.vertex_pos = c;
        }

        fragment: fn() {
            self.fb0 = self.pixel();
        }

        pixel: fn() {
            let wpp = self.cw / self.focal_px;
            if self.face > 1.5 && self.face < 2.5 {
                let ex = min(self.local.x, 1.0 - self.local.x) * self.box_size.x;
                let ez = min(self.local.z, 1.0 - self.local.z) * self.box_size.z;
                let e = min(ex, ez);
                let rim = max(self.rim.w, wpp * 1.5);
                let k = 1.0 - smoothstep(rim * 0.5, rim, e);
                return vec4(mix(self.top.rgb, self.rim.rgb, k), 1.0);
            }
            let mut side = 0.5;
            if self.face < 1.5 {
                side = 0.62;
            }
            let shade = side * mix(0.45, 1.0, self.local.y);
            let lip = 1.0 - smoothstep(0.0, max(wpp * 1.5, 0.3), (1.0 - self.local.y) * self.box_size.y);
            return vec4(mix(self.top.rgb * shade, self.rim.rgb * 0.8, lip), 1.0);
        }
    }

    mod.draw.DrawTile = mod.std.set_type_default() do #(DrawTile::script_shader(vm)){
        alpha_blend: false
        // QuadGeom winds clockwise after its y coordinate becomes world z.
        backface_culling: false
        vertex_pos: vertex_position(vec4f)
        fb0: fragment_output(0, vec4f)
        draw_call: uniform_buffer(draw.DrawCallUniforms)
        draw_pass: uniform_buffer(draw.DrawPassUniforms)
        draw_list: uniform_buffer(draw.DrawListUniforms)
        geom: vertex_buffer(geom.QuadVertex, geom.QuadGeom)

        vp0: uniform(vec4(1.0, 0.0, 0.0, 0.0))
        vp1: uniform(vec4(0.0, 1.0, 0.0, 0.0))
        vp2: uniform(vec4(0.0, 0.0, 1.0, 0.0))
        vp3: uniform(vec4(0.0, 0.0, 0.0, 1.0))
        focal_px: uniform(1000.0)

        minimap: texture_2d(float)

        tex: varying(vec2f)
        lp: varying(vec2f)
        cw: varying(float)

        clip: fn(p: vec3) -> vec4 {
            return self.vp0 * p.x + self.vp1 * p.y + self.vp2 * p.z + self.vp3;
        }

        vertex: fn() {
            let p = vec3(
                self.tile.x + self.geom.pos.x * self.tile.z,
                self.elev,
                self.tile.y + self.geom.pos.y * self.tile.w
            );
            self.tex = mix(self.uv.xy, self.uv.zw, self.geom.pos);
            self.lp = self.geom.pos * self.tile.zw;
            let c = self.clip(p);
            self.cw = c.w;
            self.vertex_pos = vec4(c.x, c.y, c.z - 0.00002 * c.w, c.w);
        }

        fragment: fn() {
            self.fb0 = self.pixel();
        }

        pixel: fn() {
            // Line height and world units per pixel, exact per fragment.
            let px = 2.0 * self.focal_px / self.cw;
            let wpp = self.cw / self.focal_px;
            let s = self.minimap.sample_as_bgra(self.tex);
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

    mod.draw.DrawGlyph = mod.std.set_type_default() do #(DrawGlyph::script_shader(vm)){
        alpha_blend: true
        backface_culling: false
        vertex_pos: vertex_position(vec4f)
        fb0: fragment_output(0, vec4f)
        draw_call: uniform_buffer(draw.DrawCallUniforms)
        draw_pass: uniform_buffer(draw.DrawPassUniforms)
        draw_list: uniform_buffer(draw.DrawListUniforms)
        geom: vertex_buffer(geom.QuadVertex, geom.QuadGeom)

        vp0: uniform(vec4(1.0, 0.0, 0.0, 0.0))
        vp1: uniform(vec4(0.0, 1.0, 0.0, 0.0))
        vp2: uniform(vec4(0.0, 0.0, 1.0, 0.0))
        vp3: uniform(vec4(0.0, 0.0, 0.0, 1.0))
        focal_px: uniform(1000.0)

        font: texture_2d(float)

        tex: varying(vec2f)
        cw: varying(float)

        clip: fn(p: vec3) -> vec4 {
            return self.vp0 * p.x + self.vp1 * p.y + self.vp2 * p.z + self.vp3;
        }

        vertex: fn() {
            let p = vec3(
                self.gpos.x + self.geom.pos.x * self.gsize.x,
                self.gpos.y,
                self.gpos.z + self.geom.pos.y * self.gsize.y
            );
            self.tex = (self.cell + self.geom.pos) / vec2(16.0, 6.0);
            let c = self.clip(p);
            self.cw = c.w;
            self.vertex_pos = vec4(c.x, c.y, c.z - 0.00005 * c.w, c.w);
        }

        fragment: fn() {
            self.fb0 = self.pixel();
        }

        pixel: fn() {
            let px = self.gsize.y * self.focal_px / self.cw;
            let d = self.font.sample_as_bgra(self.tex).x;
            // SDF spread: 4 of the 64 texels per cell map onto 0.5 of range.
            let w = clamp(64.0 / max(px, 0.001) * 0.125 * 0.5, 0.02, 0.5);
            let a = smoothstep(0.5 - w, 0.5 + w, d) * smoothstep(self.fade.x, self.fade.y, px);
            return vec4(self.color.rgb * a, a);
        }
    }

    mod.widgets.CodeScapeBase = #(CodeScape::register_widget(vm))

    mod.widgets.CodeScape = set_type_default() do mod.widgets.CodeScapeBase{
        width: Fill
        height: Fill
        font_file: crate_resource("makepad_widgets:resources/LiberationMono-Regular.ttf")
        draw_box: mod.draw.DrawBox{}
        draw_tile: mod.draw.DrawTile{}
        draw_glyph: mod.draw.DrawGlyph{}
        draw_scene_texture: mod.draw.DrawSceneTexture{}
        clear_color: #x0A1128
        draw_panel: mod.draw.DrawColor{ color: #x0A1128D8 }
        draw_title: mod.draw.DrawText{
            color: #x00B4D8
            text_style: theme.font_bold{ font_size: 15.0 }
        }
        draw_text: mod.draw.DrawText{
            color: #xF5F7FA
            text_style: theme.font_code{ font_size: 10.0 }
        }
        draw_dim: mod.draw.DrawText{
            color: #xA1A7AE
            text_style: theme.font_code{ font_size: 9.0 }
        }
        draw_caption: mod.draw.DrawText{
            color: #xF5F7FA
            text_style: theme.font_bold{ font_size: 20.0 }
        }
    }
}

#[derive(Script, ScriptHook)]
#[repr(C)]
pub struct DrawSceneTexture {
    #[deref]
    draw_super: DrawQuad,
}

impl DrawSceneTexture {
    fn set_texture(&mut self, texture: &Texture) {
        self.draw_super.draw_vars.set_texture(0, texture);
    }
}

macro_rules! draw_shader_impl {
    ($ty:ident) => {
        #[allow(dead_code)]
        impl $ty {
            pub fn begin(&mut self, cx: &mut Cx2d, view: &View) {
                let m = &view.view_proj;
                self.draw_vars
                    .set_uniform(cx.cx.cx, live_id!(vp0), &m[0..4]);
                self.draw_vars
                    .set_uniform(cx.cx.cx, live_id!(vp1), &m[4..8]);
                self.draw_vars
                    .set_uniform(cx.cx.cx, live_id!(vp2), &m[8..12]);
                self.draw_vars
                    .set_uniform(cx.cx.cx, live_id!(vp3), &m[12..16]);
                self.draw_vars
                    .set_uniform(cx.cx.cx, live_id!(focal_px), &[view.focal_px]);
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

#[derive(Script, ScriptHook)]
#[repr(C)]
pub struct DrawBox {
    #[rust]
    pub many_instances: Option<ManyInstances>,
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

#[derive(Script, ScriptHook)]
#[repr(C)]
pub struct DrawTile {
    #[rust]
    pub many_instances: Option<ManyInstances>,
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

#[derive(Script, ScriptHook)]
#[repr(C)]
pub struct DrawGlyph {
    #[rust]
    pub many_instances: Option<ManyInstances>,
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

fn rgba(c: [f32; 3], a: f32) -> Vec4 {
    vec4(c[0], c[1], c[2], a)
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Pan,
    Orbit,
}

#[derive(Script, ScriptHook, Widget)]
pub struct CodeScape {
    #[uid]
    uid: WidgetUid,
    #[source]
    source: ScriptObjectRef,
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
    draw_scene_texture: DrawSceneTexture,
    #[live]
    clear_color: Vec4,
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
    font_file: Option<ScriptHandleRef>,

    #[new]
    scene_pass: DrawPass,
    #[new]
    scene_draw_list: DrawList2d,
    #[new]
    scene_color: Texture,
    #[new]
    scene_depth: Texture,
    #[rust(false)]
    render_initialized: bool,

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
    trace: Option<Trace>,
    #[rust]
    opts: Options,
    #[rust]
    frame_index: usize,
    #[rust]
    pending_capture: Option<PendingCapture>,
    #[rust]
    glyph_buf: Vec<f32>,
}

struct PendingCapture {
    ticket: ReadbackTicket,
    path: PathBuf,
    quit_after: bool,
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
    /// A revision or `a..b` range to mark against the map.
    diff: Option<String>,
    /// A pull request to mark, read through `gh`.
    pr: Option<u32>,
    /// A Quint ITF counterexample to step through on the spec that produced it.
    trace: Option<PathBuf>,
    /// Trace steps per second; unset keeps the default hold per step.
    trace_rate: Option<f64>,
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
                "--diff" => o.diff = args.next(),
                "--pr" => o.pr = args.next().and_then(|v| v.parse().ok()),
                "--trace" => o.trace = args.next().map(PathBuf::from),
                "--trace-rate" => {
                    o.trace_rate = args
                        .next()
                        .and_then(|v| v.parse().ok())
                        .filter(|r: &f64| *r > 0.0)
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
        // A trace records its own playback, parked on the spec; anything else
        // records the tour.
        if o.record.is_some() && o.trace.is_none() {
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

/// Highest mip level of the minimap: level 0 plus four halvings.
const MIP_LEVELS: usize = 4;

/// Metal uploads a CPU-built chain level by level and fills nothing past the
/// data it is given; OpenGL and Vulkan generate the chain on the GPU from
/// level 0 alone. Makepad draws the same line in its private
/// `backend_uploads_cpu_mip_chain` (draw/src/image_cache.rs); re-check it when
/// bumping the pin.
const CPU_MIP_LEVELS: usize = if cfg!(target_vendor = "apple") {
    MIP_LEVELS
} else {
    0
};

fn upload(cx: &mut Cx, mut img: Image, mips: bool) -> Texture {
    let format = if mips {
        atlas::append_mip_levels(&mut img, CPU_MIP_LEVELS);
        TextureFormat::VecMipBGRAu8_32 {
            width: img.width,
            height: img.height,
            data: Some(img.data),
            max_level: Some(MIP_LEVELS),
            wrap: TextureWrap::ClampToEdge,
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
    fn ensure_render_targets(&mut self, cx: &mut Cx) {
        if self.render_initialized {
            return;
        }
        self.render_initialized = true;
        self.scene_color = Texture::new_with_format(
            cx,
            TextureFormat::RenderBGRAu8 {
                size: TextureSize::Auto,
                initial: true,
            },
        );
        self.scene_depth = Texture::new_with_format(
            cx,
            TextureFormat::DepthD32 {
                size: TextureSize::Auto,
                initial: true,
            },
        );
        self.scene_pass.set_color_texture(
            cx,
            &self.scene_color,
            DrawPassClearColor::ClearWith(self.clear_color),
        );
        self.scene_pass.set_depth_texture(
            cx,
            &self.scene_depth,
            DrawPassClearDepth::ClearWith(1.0),
        );
    }

    fn queue_capture(&mut self, cx: &mut Cx, path: PathBuf, quit_after: bool) -> bool {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if let Err(error) = std::fs::create_dir_all(parent) {
                log!(
                    "codescape: could not create capture directory {}: {error}",
                    parent.display()
                );
                cx.quit();
                return false;
            }
        }
        match self
            .scene_color
            .read_back(cx, ReadbackRequest { next_render: false })
        {
            Ok(ticket) => {
                self.pending_capture = Some(PendingCapture {
                    ticket,
                    path,
                    quit_after,
                });
                true
            }
            Err(error) => {
                log!("codescape: could not capture {}: {error}", path.display());
                cx.quit();
                false
            }
        }
    }

    fn drain_captures(&mut self, cx: &mut Cx) {
        for frame in cx.try_take_texture_readbacks() {
            let Some(pending) = self.pending_capture.take() else {
                continue;
            };
            if frame.ticket != pending.ticket {
                self.pending_capture = Some(pending);
                continue;
            }
            let result = frame
                .data
                .map_err(|error| error.to_string())
                .and_then(|pixels| {
                    let rgba = rgba_pixels(
                        frame.width,
                        frame.height,
                        frame.stride,
                        &pixels,
                        frame.channel_order == ReadbackChannelOrder::Bgra,
                        frame.origin == ReadbackOrigin::BottomLeft,
                    )
                    .ok_or_else(|| "readback returned invalid dimensions".to_string())?;
                    let png =
                        Cx::encode_rgba_as_png(frame.width as u32, frame.height as u32, &rgba)?;
                    std::fs::write(&pending.path, png).map_err(|error| error.to_string())
                });
            if let Err(error) = result {
                log!(
                    "codescape: could not save {}: {error}",
                    pending.path.display()
                );
                cx.quit();
                return;
            }
            if pending.quit_after {
                cx.quit();
            } else {
                self.request_frame(cx);
            }
        }
    }

    fn quit_after_capture(&mut self, cx: &mut Cx) {
        if let Some(pending) = self.pending_capture.as_mut() {
            pending.quit_after = true;
        } else {
            cx.quit();
        }
    }

    /// The monospace TTF the glyph atlas is rasterised from, declared in the
    /// script as a `crate_resource` of `makepad_widgets` and read through the
    /// script resource table (`Cx::load_script_resource` fills it on demand).
    fn bundled_font(&self, cx: &mut Cx) -> Option<std::rc::Rc<Vec<u8>>> {
        let res = self.font_file.as_ref()?;
        let (heap_key, handle) = (res.heap_key(), res.as_handle());
        if let Some(data) = cx.get_resource(heap_key, handle) {
            return Some(data);
        }
        cx.load_script_resource(heap_key, handle);
        cx.get_resource(heap_key, handle)
    }

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
        let overlay = self.build_overlay(&root, &sources);
        let mut layout = Layout::build(&sources, &repo);
        let palette = syntax_palette();
        let (minimap, texel) = atlas::build_minimap(
            &mut layout,
            &sources,
            &palette,
            &overlay,
            self.opts.atlas_size,
            CPU_MIP_LEVELS,
        );
        let ttf = self.bundled_font(cx).expect("bundled monospace font");
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
            overlay,
            glyph_on: vec![false; n],
            glyph_count: 0,
        });
        if let Some(path) = self.opts.trace.clone() {
            self.load_trace(&root, &path);
        }
    }

    /// Loads a Quint ITF counterexample and parks the camera on its spec.
    fn load_trace(&mut self, root: &Path, path: &Path) {
        let full = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let Ok(text) = std::fs::read_to_string(&full) else {
            log!("codescape: cannot read trace {}", full.display());
            return;
        };
        let Some(mut trace) = Trace::parse(&text) else {
            log!("codescape: {} is not an ITF trace", full.display());
            return;
        };
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        if !trace.bind(&scene.sources) {
            log!(
                "codescape: trace names {}, which is not on the map",
                trace.source
            );
            return;
        }
        let marked: usize = trace.var_lines.iter().map(|l| l.len()).sum();
        log!(
            "codescape: trace {} — {} states, {} vars over {} spec lines, status {}",
            trace.source,
            trace.states.len(),
            trace.vars.len(),
            marked,
            trace.status
        );
        let file = trace.file.unwrap_or_default();
        let tile = scene.layout.files.iter().position(|f| f.file == file);
        let height = if self.rect.h > 1.0 {
            self.rect.h as f32
        } else {
            1080.0
        };
        if let Some(t) = tile {
            self.cam = reading_view(&scene.layout, t, height);
        }
        if let Some(rate) = self.opts.trace_rate {
            trace.step_secs = 1.0 / rate;
        }
        // A recording plays the trace once and ends with it.
        trace.looping = self.opts.record.is_none();
        self.trace = Some(trace);
        self.apply_trace();
    }

    /// Marks for this run: a git diff, a pull request, or nothing.
    fn build_overlay(&self, root: &Path, sources: &[SourceFile]) -> Overlay {
        let (patch, label) = if let Some(pr) = self.opts.pr {
            (diff::patch_for_pr(root, pr), format!("PR #{pr}"))
        } else if let Some(spec) = &self.opts.diff {
            let label = if spec.contains("..") {
                spec.clone()
            } else {
                format!("{spec} -> working tree")
            };
            (diff::patch_for(root, spec), label)
        } else {
            return Overlay::empty(sources.len());
        };
        let Some(patch) = patch else {
            log!(
                "codescape: could not read the diff for {}; showing no marks",
                label
            );
            return Overlay::empty(sources.len());
        };
        let (ov, dropped) = diff::overlay(&patch, sources, label);
        let (added, removed) = ov.totals();
        log!(
            "codescape: diff {} — {} files marked, +{} -{}, {} deleted, {} marks dropped past end of file",
            ov.label,
            ov.touched(),
            added,
            removed,
            ov.deleted.len(),
            dropped
        );
        ov
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

    /// Advances trace playback and repaints the spec's marks when it steps.
    fn advance_trace(&mut self, dt: f64) {
        if self.trace.as_mut().is_some_and(|t| t.tick(dt)) {
            self.apply_trace();
        }
    }

    fn step_trace(&mut self, delta: isize) {
        if let Some(t) = self.trace.as_mut() {
            t.seek(delta);
        }
        self.apply_trace();
    }

    /// Rewrites the overlay from the trace's current step. Trace marks live
    /// only in the glyph layer: the minimap is rasterised once at load, so
    /// baking a step into it would freeze that step into the far view.
    fn apply_trace(&mut self) {
        let (Some(tr), Some(scene)) = (self.trace.as_ref(), self.scene.as_mut()) else {
            return;
        };
        let Some(file) = tr.file else { return };
        let lines = scene.sources[file].line_count();
        tr.apply(&mut scene.overlay, lines);
    }

    fn request_frame(&mut self, cx: &mut Cx) {
        self.next_frame = cx.new_next_frame();
    }

    fn animating(&self) -> bool {
        self.trace.as_ref().is_some_and(|t| t.playing)
            || self.fly.is_some()
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
                self.queue_capture(cx, shot, true);
                return;
            }
            self.area.redraw(cx);
            self.request_frame(cx);
            return;
        }
        if let Some(dir) = self.opts.record.clone() {
            // Capture the frame presented before this tick, then advance the
            // tour by a fixed step so the video is smooth at any render speed.
            if self.frame_index > 2 {
                let name = format!("f{:05}.png", self.frame_index - 3);
                if !self.queue_capture(cx, dir.join(name), false) {
                    return;
                }
            }
            self.frame_index += 1;
        }
        let step = if self.opts.record.is_some() {
            1.0 / self.opts.record_fps
        } else {
            dt
        };
        let settling = self.opts.record.is_some() && self.frame_index <= 3;
        if settling {
            // Let the first frames settle before the tour clock starts.
        } else if let Some(tour) = &mut self.tour {
            match tour.advance(step) {
                Some(c) => self.cam = c,
                None => {
                    self.tour = None;
                    if self.opts.exit_after_tour {
                        self.quit_after_capture(cx);
                    }
                }
            }
        }
        if !settling {
            self.advance_trace(step);
        }
        if self.opts.record.is_some() && self.trace.as_ref().is_some_and(|t| t.finished()) {
            self.quit_after_capture(cx);
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
        if self.animating() && self.pending_capture.is_none() {
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

        // File tiles. With an overlay active, changed files keep their rim and
        // everything else recedes, so the shape of the change reads from above.
        let marked = scene.overlay.touched() > 0;
        self.draw_tile.begin(cx, view);
        for (i, f) in layout.files.iter().enumerate() {
            let t = f.tile;
            self.draw_tile.tile = vec4(t.x, t.z, t.w, t.h);
            self.draw_tile.elev = f.y;
            self.draw_tile.uv = vec4(f.uv[0], f.uv[1], f.uv[2], f.uv[3]);
            match &scene.overlay.files[f.file] {
                Some(fo) => {
                    let (c, base) = match fo.state {
                        FileState::Added => (hex(0x32D74B), 0.55),
                        FileState::Modified => (hex(0xF2A33C), 0.45),
                        FileState::Deleted => (hex(0xE5484D), 0.45),
                    };
                    // Churn lifts the rim, so a big change is brighter than a typo.
                    let a = base + (fo.churn() as f32 / 400.0).min(1.0) * 0.45;
                    self.draw_tile.bg = vec4(0.05, 0.062, 0.115, 1.0);
                    self.draw_tile.rim = rgba(c, a);
                }
                None if marked => {
                    self.draw_tile.bg = vec4(0.022, 0.03, 0.062, 1.0);
                    self.draw_tile.rim = rgba(accent(f.dir), 0.1);
                }
                None => {
                    self.draw_tile.bg = vec4(0.035, 0.05, 0.1, 1.0);
                    self.draw_tile.rim = rgba(accent(f.dir), 0.35);
                }
            }
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
        } else if self.animating() {
            format!(
                "{:>3.0} fps   {:>7} glyphs   {} tiles",
                self.fps,
                scene.glyph_count,
                l.files.len()
            )
        } else {
            // The counter only runs while something animates; between
            // animations its last value would read as the frame rate.
            format!(
                "   idle   {:>7} glyphs   {} tiles",
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

        if let Some(tr) = self.trace.as_ref() {
            let mut body = format!(
                "{}\nstep {} of {}   {}",
                tr.source,
                tr.step + 1,
                tr.states.len(),
                if tr.playing { "playing" } else { "paused" }
            );
            for (line, changed) in tr.state_lines() {
                body.push_str(if changed { "\n> " } else { "\n  " });
                body.push_str(&line);
            }
            let rows = 2.0 + tr.vars.len() as f64;
            self.draw_panel.draw_abs(
                cx,
                Rect {
                    pos: dvec2(x + 16.0, y + 112.0),
                    size: dvec2(420.0, 26.0 + rows * 17.0),
                },
            );
            self.draw_title
                .draw_abs(cx, dvec2(x + 30.0, y + 122.0), "quint counterexample");
            self.draw_dim
                .draw_abs(cx, dvec2(x + 30.0, y + 148.0), &body);
        } else if scene.overlay.touched() > 0 || !scene.overlay.deleted.is_empty() {
            let ov = &scene.overlay;
            let (added, removed) = ov.totals();
            let mut body = format!(
                "{}\n{} files changed   +{}  -{}",
                ov.label,
                ov.touched(),
                added,
                removed
            );
            // The busiest files, which is what a reviewer wants to find first.
            let mut top: Vec<(usize, &crate::overlay::FileOverlay)> = ov
                .files
                .iter()
                .enumerate()
                .filter_map(|(i, f)| f.as_ref().map(|f| (i, f)))
                .collect();
            top.sort_by_key(|(_, f)| std::cmp::Reverse(f.churn()));
            for (i, f) in top.iter().take(6) {
                body.push_str(&format!(
                    "\n{} +{:<5} -{:<5} {}",
                    f.state.tag(),
                    f.added,
                    f.removed,
                    scene.sources[*i].path
                ));
            }
            for (path, removed) in ov.deleted.iter().take(3) {
                body.push_str(&format!("\nD       -{:<5} {}", removed, path));
            }
            let rows = 2.0 + top.len().min(6) as f64 + ov.deleted.len().min(3) as f64;
            self.draw_panel.draw_abs(
                cx,
                Rect {
                    pos: dvec2(x + 16.0, y + 112.0),
                    size: dvec2(560.0, 26.0 + rows * 17.0),
                },
            );
            self.draw_title
                .draw_abs(cx, dvec2(x + 30.0, y + 122.0), "changed files");
            self.draw_dim
                .draw_abs(cx, dvec2(x + 30.0, y + 148.0), &body);
        }

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

impl Widget for CodeScape {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event, _scope: &mut Scope) {
        if !self.opts.parsed {
            self.opts = Options::parse();
        }
        if let Event::Signal = event {
            self.drain_captures(cx);
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
            // Makepad 2.0 on macOS sends a zero-delta scroll for every trackpad
            // contact (`ScrollPhase::Touched`), so plain pointer movement would
            // otherwise zoom out towards the cursor. Only a real delta zooms.
            Hit::FingerScroll(fs) if fs.scroll.x != 0.0 || fs.scroll.y != 0.0 => {
                let amount = if fs.scroll.y != 0.0 {
                    fs.scroll.y
                } else {
                    fs.scroll.x
                } as f32;
                // Wheels step per notch (Makepad's X11 speed curve is erratic);
                // trackpads zoom continuously. 2.0 reports every scroll as a
                // mouse, so tell them apart by phase: only a wheel has none.
                let f = if matches!(fs.phase, makepad_widgets::event::ScrollPhase::None) {
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
                KeyCode::Space if self.trace.is_some() => {
                    if let Some(t) = self.trace.as_mut() {
                        t.playing = !t.playing;
                    }
                    self.request_frame(cx);
                    self.area.redraw(cx);
                }
                KeyCode::Comma if self.trace.is_some() => {
                    self.step_trace(-1);
                    self.area.redraw(cx);
                }
                KeyCode::Period if self.trace.is_some() => {
                    self.step_trace(1);
                    self.area.redraw(cx);
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
        if r.size.x <= 1.0 || r.size.y <= 1.0 {
            return DrawStep::done();
        }
        self.ensure_render_targets(cx.cx);
        if self.scene.is_none() {
            self.load(cx);
            self.last_time = 0.0;
            self.request_frame(cx);
            // Keys (T, H, WASD) need focus; without this they only work
            // after the first click into the map.
            cx.set_key_focus(self.area);
        }
        let view = self.view();

        self.scene_pass.set_size(cx, r.size);
        self.scene_pass.set_color_texture(
            cx,
            &self.scene_color,
            DrawPassClearColor::ClearWith(self.clear_color),
        );
        self.scene_pass.set_depth_texture(
            cx,
            &self.scene_depth,
            DrawPassClearDepth::ClearWith(1.0),
        );
        cx.make_child_pass(&self.scene_pass);
        // Captures are 1920x1080 rather than Retina-sized, and remain within
        // Makepad's bounded readback queue. Interactive rendering keeps the
        // display's native density.
        let capture_dpi = (self.opts.record.is_some() || self.opts.shot.is_some()).then_some(1.0);
        cx.begin_pass(&self.scene_pass, capture_dpi);
        self.scene_draw_list.begin_always(cx);
        cx.begin_root_turtle(r.size, makepad_widgets::Layout::flow_overlay());
        self.draw_scene(cx, &view);
        let parent_rect = self.rect;
        self.rect.x = 0.0;
        self.rect.y = 0.0;
        self.draw_hud(cx);
        self.rect = parent_rect;
        cx.end_pass_sized_turtle();
        self.scene_draw_list.end(cx);
        cx.end_pass(&self.scene_pass);

        self.draw_scene_texture.set_texture(&self.scene_color);
        self.draw_scene_texture.draw_abs(cx, r);
        self.area = self.draw_scene_texture.area();
        cx.set_pass_area(&self.scene_pass, self.area);
        DrawStep::done()
    }
}
