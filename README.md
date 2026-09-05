# xisf2png

Batch-convert **XISF** (PixInsight) and **FITS** astronomical images to
**PNG**, optionally resized to 4K with the file name stamped in the corner.
Comes as a command-line tool and a small desktop app, both pure Rust with no
runtime dependencies, for Windows, macOS (universal Intel + Apple Silicon) and
Linux.

Each image's full data range is linearly scaled to 0–255 (a plain min/max
stretch — no STF/MTF astronomical stretch). Only the first image in a file is
converted. Mono and RGB images are supported.

## Install

Prebuilt binaries for every platform are attached to each
[GitHub Release](https://github.com/peterbuitho/xisf2png/releases). Each
archive contains both the CLI (`xisf2png`) and the GUI (`xisf2png-gui`).

| Asset                             | Runs on                                                    |
| --------------------------------- | ---------------------------------------------------------- |
| `xisf2png-windows-x86_64.zip`     | Windows 10/11, 64-bit                                      |
| `xisf2png-macos-universal.zip`    | macOS 11+, Intel and Apple Silicon. Contains `xisf2png.app` (double-clickable) and the `xisf2png` CLI. |
| `xisf2png-linux-x86_64.tar.gz`    | x86-64 Linux. CLI is static (any distro); GUI needs a Wayland (or X11) desktop. |
| `xisf2png-linux-aarch64.tar.gz`   | ARM64 Linux (Raspberry Pi 4/5, ARM servers), same notes.  |

Unpack and put the binaries somewhere on your `PATH` (and, on macOS, drag
`xisf2png.app` to Applications).

**macOS note:** the app is ad-hoc signed but not notarized, so the first launch
shows an "unidentified developer" warning. Right-click the app, choose *Open*,
and confirm once. For the CLI binary, clear the quarantine flag instead:
`xattr -d com.apple.quarantine xisf2png`.

**Linux note:** the GUI's folder picker uses the desktop portal
(`xdg-desktop-portal`), which GNOME, KDE, and most other desktops provide out of
the box. It runs natively on Wayland and falls back to X11.

## GUI

Run `xisf2png-gui` (or open `xisf2png.app` on macOS). Pick the input folder,
optionally an output folder and a stamp font, tick the options you want, and
press **Convert**. Progress and a per-file log stream in as the batch runs;
**Cancel** stops after the current file.

## Command line

```
xisf2png [input_dir] [output_dir] [--recursive|-r] [--overwrite] [--resize4k] [--font <file>]
xisf2png [input_dir] [output_dir] --png-only [--recursive|-r] [--overwrite] [--font <file>]
```

Every `.xisf`, `.fits`, `.fit` and `.fts` file found is converted. If
`input_dir` is omitted, the current folder is used. If `output_dir` is
omitted, PNGs are written next to their source files (or, with `--png-only`,
edited in place).

| Option              | Meaning                                                     |
| ------------------- | ----------------------------------------------------------- |
| `-r`, `--recursive` | Recurse into subfolders; the output tree mirrors the input. |
| `--overwrite`       | Overwrite existing `.png` files (default: skip them).       |
| `--resize4k`        | Scale each PNG (up or down, aspect ratio kept) to cover 3840×2160, then center-crop to exactly 3840×2160 — no padding — and stamp the file name in the bottom-right corner (48 px, white with a soft shadow). |
| `--png-only`        | Skip XISF/FITS conversion entirely: pick up existing `.png` files in `input_dir` and only run the `--resize4k` step on them (implies `--resize4k`). With no `output_dir` the PNGs are modified in place; with one they are copied there first, honouring `--overwrite`. |
| `--font <file>`     | A `.ttf` / `.otf` font file for the file-name stamp. Default: the bundled DejaVu Sans Condensed Bold. `--font=<file>` also works. |
| `-V`, `--version`   | Print the version.                                          |
| `-h`, `--help`      | Show help.                                                  |

Per-file output lines: `OK <path>`, `SKIP <path>`, `ERROR <path>: <reason>`.
A bad file is reported and the batch continues. Exit code is `1` if any file
failed, `2` for a usage error, `0` otherwise.

Examples:

```
xisf2png "D:\captures" "D:\previews" --recursive
xisf2png "D:\previews" --png-only --recursive
xisf2png ~/astro/previews --png-only --font ~/Library/Fonts/Futura-CondensedBold.ttf
xisf2png --png-only                      # PNGs in the current folder, in place
```

## Format support

**XISF** (monolithic `.xisf`, version 1.0):

- Sample formats: `UInt8/16/32/64`, `Float32/64` (not `Complex`).
- Pixel storage: planar and interleaved (`Normal`); little- and big-endian.
- Data location: `attachment`, `embedded`, `inline` (base64 / hex).
- Compression: `zlib`, `lz4`, `lz4hc`, `zstd`, with or without byte-shuffle
  (`+sh`).

**FITS** (`.fits`, `.fit`, `.fts`), image in the primary HDU:

- `BITPIX` 8, 16, 32, 64 (integers, with `BZERO`/`BSCALE` applied, so the
  usual unsigned-16-bit camera files work) and −32, −64 (floats).
- `NAXIS` 2 (mono) or 3 with `NAXIS3` = 1 or 3 (RGB planes).
- Row order: FITS stores the bottom row first, so images are flipped to
  display orientation by default. Files that carry `ROWORDER = 'TOP-DOWN'`
  (N.I.N.A., Siril and others) are left as-is.
- Not supported: images stored in extensions, tile-compressed `.fz` files,
  and `.fits.gz`.

## Build from source

Requires a Rust toolchain (install via [rustup](https://rustup.rs)). The code
and all dependencies are pure Rust, so the same command builds both binaries
on Windows, macOS and Linux:

```
cargo build --release
```

Binaries land in `target/release/` as `xisf2png` and `xisf2png-gui`
(`.exe` on Windows). To build just one: `cargo build --release --bin xisf2png`.

On Linux, the GUI needs two small development packages at build time (it
loads the real libraries dynamically at run time):

```
sudo apt install libwayland-dev libxkbcommon-dev      # Debian / Ubuntu
sudo dnf install wayland-devel libxkbcommon-devel     # Fedora
```

On Windows without the MSVC Build Tools, use the GNU toolchain instead:

```
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release
```

### Building for other targets

Add a target with `rustup target add <triple>` and pass `--target <triple>`
to `cargo build`. Useful triples:

- `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl` — fully static
  Linux CLI binaries (what the releases use for `xisf2png`; the GUI is built
  for the normal `-gnu` targets because it needs the system graphics stack).
- `x86_64-apple-darwin` + `aarch64-apple-darwin` — on a Mac, build both and
  merge into one universal binary:
  ```
  lipo -create -output xisf2png \
    target/x86_64-apple-darwin/release/xisf2png \
    target/aarch64-apple-darwin/release/xisf2png
  ```

Building macOS binaries requires a Mac. The simplest way to get every
platform, including the macOS `.app` bundle, is the release workflow below.

### Releasing

[`.github/workflows/release.yml`](.github/workflows/release.yml) builds all
four archives above on GitHub's Windows, macOS and Linux runners, assembles
the macOS `.app` bundle (icon, `Info.plist`, ad-hoc code signature), and
attaches everything to a GitHub Release. To cut a release, tag a commit and
push the tag:

```
git tag v0.2.0
git push origin v0.2.0
```

To check that everything still builds without publishing anything, run the
workflow manually from the repo's **Actions** tab; the archives are then
available as workflow artifacts.

## Dependencies

All pure Rust (no C toolchain needed):

| Crate       | Purpose                                     |
| ----------- | ------------------------------------------- |
| `roxmltree` | XISF XML header parsing                     |
| `base64`    | inline / embedded base64                    |
| `hex`       | inline / embedded hex                       |
| `flate2`    | zlib decompression                          |
| `lz4_flex`  | lz4 / lz4hc decompression                   |
| `ruzstd`    | zstd decompression                          |
| `image`     | PNG encoding/decoding, 4K resize            |
| `ab_glyph`  | rasterising the file-name stamp             |
| `eframe`    | GUI (egui, OpenGL via glow, Wayland/X11)    |
| `rfd`       | native folder / file dialogs                |

The stamp font, DejaVu Sans Condensed Bold, is embedded in the binary; its
license is in [`assets/fonts/LICENSE-DejaVu.txt`](assets/fonts/LICENSE-DejaVu.txt).
