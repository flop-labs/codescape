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

## Run

```sh
cd tools/codescape
cargo run --release                  # repo containing the current directory
cargo run --release -- /path/to/repo # any git checkout
cargo run --release -- --tour --loop # scripted flight, repeating
```

Linux needs the X11, GL, ALSA and PulseAudio dev libraries, as for any
Makepad app (`libx11-dev libxcursor-dev libgl-dev libegl-dev libasound2-dev
libpulse-dev`). macOS and Windows need nothing extra.

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

The scan covers tracked source, docs and config (`git ls-files`). It skips
lockfiles, bundles, minified files, files over 20k lines, and files whose
average line is over 240 characters.

## Recording

`--record DIR` runs the tour at a fixed step (`--record-fps`, default 30). It
saves one `xwd` screenshot of the window per frame and exits when the tour
ends, so the video stays smooth however fast the machine renders.
`--at SECONDS --shot FILE` saves a single frame from that point in the tour.
Both call `xwd`, so they need an X server; Xvfb with Mesa llvmpipe works:

```sh
Xvfb :99 -screen 0 2400x1400x24 & export DISPLAY=:99
target/release/codescape --record /tmp/frames
ffmpeg -framerate 30 -i /tmp/frames/f%05d.xwd -c:v libx264 -pix_fmt yuv420p -crf 20 codescape.mp4
```
