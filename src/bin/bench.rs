//! Headless benchmark for the parts of codescape that decide how it scales:
//! the load pipeline (scan, layout, minimap raster) and the per-frame glyph
//! collection that runs on the CPU for every frame the camera moves.
//!
//! It never opens a window, so it runs on a machine with no GPU and no X
//! server, and it reports the memory the renderer would be holding.
//!
//! Usage: bench [REPO] [--repeat N] [--frames N] [--atlas N] [--width W] [--height H]

use codescape::{
    atlas, diff, layout::Layout, mem, overlay::Overlay, scan, scene, tour::Tour, trace::Trace,
};
use std::path::PathBuf;
use std::time::Instant;

fn mb(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn main() {
    let mut repo: Option<PathBuf> = None;
    let (mut repeat, mut frames, mut atlas_size) = (1usize, 600usize, 8192usize);
    let (mut width, mut height) = (1920.0f32, 1080.0f32);
    let (mut diff_spec, mut trace_path): (Option<String>, Option<String>) = (None, None);
    let mut pr: Option<u32> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut next = |d: f64| args.next().and_then(|v| v.parse().ok()).unwrap_or(d);
        match a.as_str() {
            "--repeat" => repeat = next(1.0) as usize,
            "--frames" => frames = next(600.0) as usize,
            "--atlas" => atlas_size = next(8192.0) as usize,
            "--diff" => diff_spec = args.next(),
            "--pr" => pr = args.next().and_then(|v| v.parse().ok()),
            "--trace" => trace_path = args.next(),
            "--width" => width = next(1920.0) as f32,
            "--height" => height = next(1080.0) as f32,
            s if !s.starts_with('-') => repo = Some(PathBuf::from(s)),
            _ => {}
        }
    }
    let root = repo.unwrap_or_else(|| std::env::current_dir().unwrap());

    let t = Instant::now();
    let mut sources = scan::scan_repo(&root);
    let scan_ms = t.elapsed().as_secs_f64() * 1e3;
    let scanned = sources.len();

    // A synthetic multiple of the repo, to see how the pipeline scales with
    // size on real source rather than on generated text.
    if repeat > 1 {
        let base: Vec<_> = (0..scanned).collect();
        for k in 1..repeat {
            for &i in &base {
                let mut s = scan::SourceFile {
                    path: format!("copy{k}/{}", sources[i].path),
                    text: sources[i].text.clone(),
                    class: sources[i].class.clone(),
                    line_starts: sources[i].line_starts.clone(),
                };
                s.path.shrink_to_fit();
                sources.push(s);
            }
        }
    }
    let text_bytes: usize = sources
        .iter()
        .map(|s| s.text.len() * 2 + s.line_starts.len() * 4)
        .sum();
    let lines: usize = sources.iter().map(|s| s.line_count()).sum();

    let t = Instant::now();
    let mut layout = Layout::build(&sources, "bench");
    let layout_ms = t.elapsed().as_secs_f64() * 1e3;

    // Exercise the real git -> overlay path, which is otherwise only reachable
    // through the renderer.
    let overlay = match diff_spec.clone().or(pr.map(|n| format!("PR #{n}"))) {
        Some(spec) => {
            let t = Instant::now();
            let patch = match pr {
                Some(n) => diff::patch_for_pr(&root, n).expect("gh pr diff"),
                None => diff::patch_for(&root, &spec).expect("git diff"),
            };
            let (ov, dropped) = diff::overlay(&patch, &sources, spec.clone());
            let (a, r) = ov.totals();
            let marked: usize = ov
                .files
                .iter()
                .flatten()
                .map(|f| f.marks.iter().filter(|m| **m != 0).count())
                .sum();
            println!(
                "diff            {} — {} files, +{} -{}, {} deleted, {} lines marked, {} dropped, {:.0} ms ({:.0} KB patch)",
                spec,
                ov.touched(),
                a,
                r,
                ov.deleted.len(),
                marked,
                dropped,
                t.elapsed().as_secs_f64() * 1e3,
                patch.len() as f64 / 1024.0
            );
            ov
        }
        None => Overlay::empty(sources.len()),
    };
    let palette = scene::syntax_palette();
    let t = Instant::now();
    let (minimap, texel) =
        atlas::build_minimap(&mut layout, &sources, &palette, &overlay, atlas_size);
    let atlas_ms = t.elapsed().as_secs_f64() * 1e3;
    let atlas_bytes = minimap.width * minimap.height * 4;
    let atlas_dims = (minimap.width, minimap.height);
    drop(minimap);

    println!("repo            {}", root.display());
    println!(
        "files           {} ({} scanned x {})",
        sources.len(),
        scanned,
        repeat
    );
    println!("lines           {:.2}M", lines as f64 / 1e6);
    println!(
        "load            scan {:.0} ms   layout {:.0} ms   minimap {:.0} ms   total {:.2} s",
        scan_ms,
        layout_ms,
        atlas_ms,
        (scan_ms + layout_ms + atlas_ms) / 1e3
    );
    println!(
        "minimap atlas   {}x{} @ texel {}  =  {:.0} MB  (+33% uploaded with mips)",
        atlas_dims.0,
        atlas_dims.1,
        texel,
        mb(atlas_bytes)
    );
    println!("source text     {:.0} MB resident", mb(text_bytes));

    // Per-frame CPU: walk the tour and collect the glyph set for each frame.
    let paths: Vec<&str> = sources.iter().map(|s| s.path.as_str()).collect();
    let mut tour = Tour::flop_core(&layout, &paths, true);
    let n = layout.files.len();
    let mut sc = scene::Scene {
        sources,
        layout,
        repo: "bench".into(),
        revision: "bench".into(),
        overlay,
        glyph_on: vec![false; n],
        glyph_count: 0,
    };
    if let Some(path) = &trace_path {
        let text = std::fs::read_to_string(root.join(path)).expect("trace file");
        let mut tr = Trace::parse(&text).expect("ITF trace");
        if !tr.bind(&sc.sources) {
            // The spec may be absent, renamed, or filtered out by the scan.
            eprintln!(
                "bench: the trace names {}, which is not among the {} scanned files",
                tr.source,
                sc.sources.len()
            );
            std::process::exit(2);
        }
        let f = tr.file.expect("bind succeeded");
        let marked: usize = tr.var_lines.iter().map(|l| l.len()).sum();
        println!(
            "trace           {} — {} states, {} vars, {} spec lines marked, status {}",
            tr.source,
            tr.states.len(),
            tr.vars.len(),
            marked,
            tr.status
        );
        for step in 0..tr.states.len() {
            tr.step = step;
            let lines = sc.sources[f].line_count();
            tr.apply(&mut sc.overlay, lines);
            let bright = (0..lines)
                .filter(|&l| sc.overlay.mark(f, l) == codescape::overlay::M_STEP)
                .count();
            let dim = (0..lines)
                .filter(|&l| sc.overlay.mark(f, l) == codescape::overlay::M_PAST)
                .count();
            let changed: Vec<&str> = tr
                .changed(step)
                .iter()
                .map(|&v| tr.vars[v].as_str())
                .collect();
            println!(
                "  step {}/{}      {} lines bright, {} dim   changed: {}",
                step + 1,
                tr.states.len(),
                bright,
                dim,
                if changed.is_empty() {
                    "-".to_string()
                } else {
                    changed.join(", ")
                }
            );
        }
    }

    let mut buf: Vec<f32> = Vec::new();
    let mut ms: Vec<f64> = Vec::with_capacity(frames);
    let mut peak_glyphs = 0usize;
    let mut total_glyphs = 0usize;
    let dt = 1.0 / 60.0;
    for _ in 0..frames {
        let Some(cam) = tour.advance(dt) else { break };
        let view = cam.view(width, height);
        buf.clear();
        let t = Instant::now();
        scene::collect_glyphs(&mut sc, &view, &mut buf);
        ms.push(t.elapsed().as_secs_f64() * 1e3);
        peak_glyphs = peak_glyphs.max(sc.glyph_count);
        total_glyphs += sc.glyph_count;
    }
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = ms.iter().sum::<f64>() / ms.len().max(1) as f64;
    let pick = |q: f64| ms[((ms.len() as f64 - 1.0) * q) as usize];
    println!(
        "glyph collect   mean {:.2} ms   p50 {:.2}   p95 {:.2}   max {:.2}   over {} tour frames at {}x{}",
        mean,
        pick(0.5),
        pick(0.95),
        pick(1.0),
        ms.len(),
        width as u32,
        height as u32
    );
    println!(
        "glyphs          peak {}   mean {}   ({:.0} MB/frame instance data at peak)",
        peak_glyphs,
        total_glyphs / ms.len().max(1),
        mb(peak_glyphs * scene::GLYPH_SLOTS * 4)
    );
    println!(
        "cpu headroom    {:.0} fps before glyph collection alone saturates one core",
        1000.0 / mean.max(1e-6)
    );
    // The tour never parks at the altitude where the most files are still
    // readable, which is where the glyph set peaks. Sweep it directly.
    let overview = scene::overview(&sc.layout);
    let near = scene::reading_view(&sc.layout, 0, height);
    let (mut worst_ms, mut worst_glyphs, mut worst_dist) = (0.0f64, 0usize, 0.0f32);
    let steps = 240;
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        // Geometric sweep from reading distance out to the whole repo.
        let dist = near.dist * (overview.dist / near.dist).powf(f);
        let cam = codescape::camera::Camera {
            target: overview.target,
            dist,
            yaw: overview.yaw,
            pitch: overview.pitch,
        };
        let view = cam.view(width, height);
        buf.clear();
        let t = Instant::now();
        scene::collect_glyphs(&mut sc, &view, &mut buf);
        let el = t.elapsed().as_secs_f64() * 1e3;
        if sc.glyph_count > worst_glyphs {
            worst_glyphs = sc.glyph_count;
            worst_ms = el;
            worst_dist = dist;
        }
    }
    println!(
        "worst frame     {} glyphs in {:.2} ms at camera distance {:.0}  ({:.0} MB instance data, {:.0} fps ceiling)",
        worst_glyphs,
        worst_ms,
        worst_dist,
        mb(worst_glyphs * scene::GLYPH_SLOTS * 4),
        1000.0 / worst_ms.max(1e-6)
    );
    match mem::resident() {
        Some(r) => println!(
            "memory          {:.0} MB resident now, {:.0} MB peak",
            mb(r.now),
            mb(r.peak)
        ),
        None => println!("memory          not readable on this platform"),
    }
}
