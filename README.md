# codescape

A 3D map of the flop-core source that you can fly through, built on
[Makepad](https://github.com/makepad/makepad). It is inspired by Rik Arends'
Makepad source visualizer.

![overview](docs/overview.jpg)

Each directory is a raised block, and nested directories stack on top of their
parent. Each file is a flat tile holding its real text, split into columns so
the tile stays roughly square. Blocks take an accent colour from their
top-level directory; larger directories get cyan, green and blue first.

It uses two levels of detail:

- **Far:** every file is one quad. It samples a mipmapped minimap atlas where
  each texel covers two characters of one line, coloured by syntax class.
  The whole repo (about 4.6k files and 1M lines) costs about 4.6k quads.
- **Near:** files that appear large on screen also get one quad per
  character, drawn from a signed-distance-field font atlas. Nearest files come
  first, up to 450k glyphs, and the text stays sharp at any zoom.

A scan and layout of flop-core takes about 0.6 s. All text is uploaded once;
the camera changes only uniforms and the near-glyph set.

## Build

The crate is split in two. The library (`src/lib.rs`) holds everything that
decides how the tool behaves and scales — scanning, layout, the minimap
rasteriser, overlays, and the scene geometry that chooses each frame's glyphs.
The binary adds the Makepad renderer on top. Nothing in the library touches
Makepad, so its tests and the benchmark build and run on a machine with no GPU,
no X server and no GUI development packages:

```sh
cargo test --lib --no-default-features      # or: just verify-codescape
cargo run  --release --no-default-features --bin bench
```

The renderer itself needs the libraries any Makepad app links against:

```sh
sudo apt-get install libx11-dev libxcursor-dev libasound2-dev libpulse-dev
```

Makepad declares its own FFI, so no C headers are involved: the only thing
those packages contribute to the build is the unversioned `libX.so` symlink
that `-lX` resolves through. `libgl-dev` and `libegl-dev` are not needed —
nothing links against GL, which is resolved at runtime through `libGL.so.1`.
macOS and Windows need no system packages. macOS does need an accepted Xcode
license, because every link goes through `xcrun`; after an Xcode update the
build fails at the first build script until `sudo xcodebuild -license accept`
has been run.

### Away from Linux, the far view aliases

This is a property of the Makepad release pinned here, `makepad-platform`
1.0.0 from crates.io, not of Makepad. In 1.0.0 the mipmapped upload format
(`VecMipBGRAu8_32`) is implemented in the OpenGL backend only: Metal's
`update_vec_texture` ends in `_=>panic!()`, and its MSL sampler is built
`sampler(mag_filter::linear, min_filter::linear)`, with no mip filter to
sample a chain with. So on macOS the minimap goes up unmipped: the tool runs
and logs a line saying so, but tiles minified at distance alias, because one
quad covers a whole file. Reading distance is unaffected — the glyph layer
does not use the minimap — so stepping a Quint trace, which parks on a single
spec, looks the same everywhere.

Makepad itself has the whole path on `dev`, and has had most of it for a
while: `73d9972` (2026-03-09, "mipmapping") added the Metal per-level upload
and the `mip_filter::linear` sampler, and #1127 (2026-07-21) added mip chains
for decoded images and fixed Metal's missing `VecMipBGRAu8_32` upload arm,
which until then allocated levels without filling them. Lifting
`MIPMAP_UPLOAD` in `src/scape.rs` therefore waits on moving off the 1.0.0
release, not on an upstream fix.

Upstream's remaining gap is a policy one and does not apply here: decoded
images only request mipmaps on Linux by default (`image_cache_use_mipmaps`,
overridable with `MAKEPAD_IMAGE_MIPMAPS=1`), because on Apple the chain is
built on the CPU during decode and costs about a third more texture memory.
The minimap is not a decoded image — it is rasterised here and uploaded as an
explicit mip texture — so it is unaffected by that default either way.

Measured on an M3 Max, that costs quality but not use. Comparing consecutive
recorded frames of the overview, tile rims, directory labels and the colour of
each district hold still, while the texel pattern inside a tile changes from
frame to frame, so the map twinkles under motion. The shape of the repository
still reads from altitude.

The backends also differ on depth. Makepad's X11 window asks EGL for no depth
buffer, so on Linux the scene simply paints in draw order. The Metal window
has a real one, cleared to 1.0 and tested less-or-equal, and every Makepad 2D
draw writes depth near 0.5. That is why the window background in `app.rs` is
a pass clear colour rather than a drawn `draw_bg`: a drawn background writes
0.5 across the whole window, the scene's remapped depth sits close to 1.0
for everything but the nearest few percent of the view, and on Metal the
entire map is hidden behind it — the HUD draws, the map does not.

The renderer-free core cross-checks for macOS without an SDK, which is the
cheapest guard against breaking it:

```sh
rustup target add aarch64-apple-darwin
cargo check --no-default-features --lib --bins --target aarch64-apple-darwin
```

The GUI target cannot be checked that way: `makepad-platform`'s build script
compiles Objective-C and needs the real macOS SDK.

## Run

```sh
cd tools/codescape
cargo run --release                  # repo containing the current directory
cargo run --release -- /path/to/repo # any git checkout
cargo run --release -- --tour --loop # scripted flight, repeating
```

| Input | Action |
|-------|--------|
| drag | pan |
| right-drag / shift-drag | orbit |
| wheel | zoom toward the cursor |
| click a file | fly in to reading distance |
| `W` `A` `S` `D` / arrows | move |
| `Q` / `E` | rotate |
| `R` / `F` | tilt |
| `Z` / `X` | zoom in / out |
| `T` | tour |
| `H` | overview |
| space | play / pause a trace |
| `,` `.` | step a trace back / forward |

The scan covers tracked source, docs and config (`git ls-files`). It skips
lockfiles, bundles, minified files, files over 20k lines, and files whose
average line is over 240 characters.

## Recording

`--record DIR` runs the tour at a fixed step (`--record-fps`, default 30). It
saves one screenshot of the window per frame and exits when the tour
ends, so the video stays smooth however fast the machine renders.
`--at SECONDS --shot FILE` saves a single frame from that point in the tour.
On Linux both call `xwd`, so they need an X server; Xvfb with Mesa llvmpipe works:

```sh
Xvfb :99 -screen 0 2400x1400x24 & export DISPLAY=:99
target/release/codescape --record /tmp/frames
ffmpeg -framerate 30 -i /tmp/frames/f%05d.xwd -c:v libx264 -pix_fmt yuv420p -crf 20 codescape.mp4
```

On macOS they call `screencapture` on the window instead and write PNG
(`f%05d.png`). Screen Recording permission belongs to whichever process
launched the tool, and every frame fails with "could not create image from
window" until that process has it — granting it to the binary itself does
nothing, because it is not an application bundle. Launch from a terminal that
already has the permission. The screen must also be awake and unlocked: a
locked screen captures as black, and a sleeping display fails outright.

## Diff: what a change touches

`--diff` marks the map with a git diff instead of showing the tree flat.
Changed files keep their rim and their syntax colours; everything else recedes,
so the shape of a change reads from altitude before any file is legible.

```sh
cargo run --release -- --diff main          # main against the working tree
cargo run --release -- --diff a1b2c3d..HEAD # an explicit range
cargo run --release -- --pr 1698            # a pull request, read through `gh`
```

Added lines are green, lines that replaced something are amber, and the line
that closed a pure deletion is red. File tiles take the same colours, with the
rim brightening with churn, so a 600-line rewrite is louder than a typo. The
inspector lists the busiest files and any file deleted outright.

`--diff <rev>` compares a revision against the **working tree**, which is
exactly what the map renders, so line numbers always agree. `--diff a..b` and
`--pr` compare two revisions instead, and then the patch can name lines the
local file does not have. Such a mark cannot be placed anywhere honest —
putting it on the last line would paint an unchanged line as changed — so it
is dropped and counted, and the count is logged. The file still reads as
changed: its tile keeps the state and the +/- totals.

Deleted files have no tile, because their text is not on disk. They are listed
in the inspector rather than drawn — placing a ghost tile would mean laying out
the union of both trees, which is a larger change than this one.

The diff is parsed from a unified patch by `src/diff.rs`, fed either by
`git diff -U0` or by `gh pr diff`, so a pull request needs no local branch.
`--pr` derives `owner/repo` from the origin URL, because this repository's
origin is a proxy scheme that `gh` cannot resolve on its own.

## Quint counterexamples

A Quint run that violates an invariant emits an ITF trace: an ordered list of
states binding every state variable. The spec that produced it is already a
tile on the map, so a trace needs no new geometry — it is the same overlay,
with marks that move as the trace is stepped.

```sh
just quint-itf formal-specs/channel/channel-payout-liveness.qnt inv_escape_naive_safe
cargo run --release -- \
  --trace formal-specs/channel/channel-payout-liveness.inv_escape_naive_safe.itf.json
```

The camera parks on the spec and plays the counterexample. Each step lights the
lines where the variables that just changed can be written; earlier steps stay
visible but dim, so the path through the spec accumulates as it runs. The
inspector shows every variable's current value, marking the ones that moved.

Quint actions must assign every variable, so a variable's name appears on
almost every action line. Identity writes (`best' = best`) are therefore
ignored: only a declaration or an assignment whose right-hand side is not the
variable itself counts as somewhere a change can come from. On
`channel-payout-liveness` that is the difference between lighting 11 lines on
every step and lighting 3 — the last of which is the `paidNaiveEscape' = true`
that breaks the invariant.

A counterexample is as short as Quint can make it — six states here. For a
long run to watch, simulate without an invariant, which keeps a random walk of
`--max-steps` states (status `ok`, not `violation`), and raise the playback
rate from its default of one step per 1.6 s with `--trace-rate` (steps per
second):

```sh
quint run formal-specs/channel/channel-payout-liveness.qnt \
  --max-steps=600 --max-samples=1 --seed=7 --backend=rust --out-itf=/tmp/long.itf.json
cargo run --release -- --trace /tmp/long.itf.json --trace-rate 60
```

With `--trace`, `--record DIR` records the trace instead of the tour: it
plays the trace once at `--trace-rate`, holds the last state for 1.5 s and
exits. 600 steps at 60 a second make an 11.5 s video (frames are PNG on
macOS; under X11 they are `f%05d.xwd`):

```sh
cargo run --release -- --trace /tmp/long.itf.json --trace-rate 60 --record /tmp/frames
ffmpeg -framerate 30 -i /tmp/frames/f%05d.png -vf scale=1920:-2 \
  -c:v libx264 -pix_fmt yuv420p -crf 20 trace.mp4
```

Trace marks live only in the near, glyph layer. The minimap atlas is rasterised
once at load, so baking a step into it would freeze that step into the far
view; diff marks, which do not change during a run, are baked into both.

## Benchmark

`bench` runs the load pipeline and the per-frame glyph collection without
opening a window, and reports what the renderer would be holding. It takes the
same `--diff` and `--trace` flags, which is how those paths are exercised
without a GPU.

```sh
cargo run --release --no-default-features --bin bench -- . --frames 12000
cargo run --release --no-default-features --bin bench -- . --repeat 8   # synthetic 8x repo
cargo run --release --no-default-features --bin bench -- . --pr 1698    # check the diff path
cargo run --release --no-default-features --bin bench -- . --trace path/to.itf.json
```

On a DGX Spark (GB10, 20-core Grace, aarch64), scanning flop-core and eight
synthetic copies of it:

| repo | files | lines | load | minimap atlas | text resident | worst frame | RSS |
|------|-------|-------|------|---------------|---------------|-------------|-----|
| 1x | 4,598 | 1.04M | 0.49 s | 8192x7488, texel 2 — 234 MB | 83 MB | 35k glyphs, 0.35 ms | 435 MB |
| 2x | 9,196 | 2.08M | 0.51 s | 8192x6720, texel 3 — 210 MB | 165 MB | 107k glyphs, 0.27 ms | 442 MB |
| 4x | 18,392 | 4.17M | 0.56 s | 8192x7424, texel 4 — 232 MB | 331 MB | 87k glyphs, 0.29 ms | 643 MB |
| 8x | 36,784 | 8.33M | 0.60 s | 8192x6784, texel 6 — 212 MB | 662 MB | 205k glyphs, 0.85 ms | 970 MB |

Two properties matter and both hold. Texture memory is **constant**: the packer
picks the finest texel from a ladder that still fits one 8192-square atlas, so
eight times the source costs the same 200-odd MB and reads coarser rather than
larger. And the glyph set is bounded by **screen area**, not by repo size: a
line must clear 3 px to get real glyphs, which confines them to a disc about
770 world units across whatever the repo contains. Resident memory grows only
with the source text itself, at roughly 80 bytes per line.

The synthetic multiples copy the scanned files in memory, so the `scan` column
is the cost of reading flop-core once and does not scale with `--repeat`.

`bench` measures no GPU time at all — it stops before the renderer. For that,
run the app itself and read the fps in the HUD.

On the same machine (NVIDIA GB10, 2400x1400, no vsync), flying the tour over
flop-core with a diff overlay holds **1.9k–2.6k fps** at 4,604 tiles and 1k–5k
glyphs, with the GPU at 89% and 32 W. Frame times of 0.4–0.5 ms mean the tool
is nowhere near GPU-bound at this repo size; what grows is tile instances,
linear in file count.

On an M3 Max (macOS 26.6, Metal, 1512x886 pt at 2x) the same tour holds
**115–128 fps**, which is the display's refresh rate: Makepad presents on
vsync, so this is a floor, not a ceiling. At that rate the GPU reports about
30% utilisation — a whole-device figure, so the tool's own share is lower —
and one CPU core is about 22% busy. Resident memory is 1.35 GB: roughly
750 MB of heap, 520 MB of GPU allocations (the 234 MB minimap atlas among
them) and 62 MB of Retina drawables.

To run it without a desktop session, a headless X server on the GPU works —
the NVIDIA Xorg driver that ships with the 580 packages plus a config with
`AllowEmptyInitialConfiguration`. Xvfb does not: it has no DRI3, so Mesa falls
back to llvmpipe and any number from it is meaningless.
