# xisf2png

Batch-convert PixInsight **XISF** astronomical images to **PNG**. Written in
Rust; builds to a single self-contained executable for Windows, macOS
(universal Intel + Apple Silicon) and Linux.

Each image's full data range is linearly scaled to 0–255 (a plain min/max
stretch — no STF/MTF astronomical stretch). Only the first `<Image>` in a file
is converted. Mono and RGB images are supported.

## Usage

```
xisf2png [input_dir] [output_dir] [--recursive|-r] [--overwrite] [--resize4k] [--font <name>]
xisf2png [input_dir] [output_dir] --png-only [--recursive|-r] [--overwrite] [--font <name>]
```

If `input_dir` is omitted, the current folder is used. If `output_dir` is
omitted, PNGs are written next to their source files (or, with `--png-only`,
edited in place).

| Option              | Meaning                                                     |
| ------------------- | ----------------------------------------------------------- |
| `-r`, `--recursive` | Recurse into subfolders; the output tree mirrors the input. |
| `--overwrite`       | Overwrite existing `.png` files (default: skip them).       |
| `--resize4k`        | Scale each PNG (up or down, aspect ratio kept) to cover 3840×2160, then center-crop to exactly 3840×2160 — no padding — and stamp the file name in the bottom-right corner (48 pt, white). Requires [ImageMagick](#imagemagick) on `PATH`; if it's missing or fails, conversion continues and a warning is printed at the end. |
| `--png-only`        | Skip XISF conversion entirely: pick up existing `.png` files in `input_dir` and only run the `--resize4k` step on them (implies `--resize4k`). With no `output_dir` the PNGs are modified in place; with one they are copied there first, honouring `--overwrite`. |
| `--font <name>`     | Font for the file-name stamp, by the name ImageMagick knows it (list them with `magick -list font`). Default: ImageMagick's standard font, which works on every platform. `--font=<name>` also works. |
| `-h`, `--help`      | Show help.                                                  |

Per-file output lines: `OK <path>`, `SKIP <path>`, `ERROR <path>: <reason>`.
A bad file is reported and the batch continues. Exit code is `1` if any file
failed, `2` for a usage error, `0` otherwise.

Examples:

```
xisf2png "D:\captures" "D:\previews" --recursive
xisf2png "D:\previews" --png-only --recursive
xisf2png ~/astro/previews --png-only --font Helvetica-Bold
xisf2png --png-only                      # PNGs in the current folder, in place
```

### ImageMagick

Only needed for `--resize4k` / `--png-only`. The tool looks for `magick`
(ImageMagick 7) and, on macOS/Linux, falls back to `convert` (ImageMagick 6,
still what Debian/Ubuntu ship). Either works.

- Windows: [installer](https://imagemagick.org/script/download.php#windows),
  or `winget install ImageMagick.ImageMagick`
- macOS: `brew install imagemagick`
- Debian/Ubuntu: `sudo apt install imagemagick` · Fedora: `sudo dnf install ImageMagick`
  · Arch: `sudo pacman -S imagemagick`

## Format support

- Sample formats: `UInt8/16/32/64`, `Float32/64` (not `Complex`).
- Pixel storage: planar and interleaved (`Normal`); little- and big-endian.
- Data location: `attachment`, `embedded`, `inline` (base64 / hex).
- Compression: `zlib`, `lz4`, `lz4hc`, `zstd`, with or without byte-shuffle
  (`+sh`).

## Install

Prebuilt binaries for every platform are attached to each
[GitHub Release](https://github.com/peterbuitho/xisf2png/releases):

| Asset                              | Runs on                                          |
| ---------------------------------- | ------------------------------------------------ |
| `xisf2png-windows-x86_64.zip`      | Windows 10/11, 64-bit                            |
| `xisf2png-macos-universal.tar.gz`  | macOS, Intel and Apple Silicon (one binary)      |
| `xisf2png-linux-x86_64.tar.gz`     | Any x86-64 Linux distro (static, no glibc dependency) |
| `xisf2png-linux-aarch64.tar.gz`    | Any ARM64 Linux (Raspberry Pi 4/5, ARM servers)  |

Unpack and put `xisf2png` (or `xisf2png.exe`) somewhere on your `PATH`. On
macOS, an unsigned binary downloaded from a browser is quarantined; clear it
once with `xattr -d com.apple.quarantine xisf2png`.

## Build from source

Requires a Rust toolchain (install via [rustup](https://rustup.rs)). The code
and all dependencies are pure Rust, so the same command works on Windows,
macOS and Linux with no C compiler or other tooling:

```
cargo build --release
```

The binary is `target/release/xisf2png` (`xisf2png.exe` on Windows) — a
single self-contained executable, no runtime needed.

On Windows without the MSVC Build Tools, use the GNU toolchain instead:

```
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release
```

### Building for other targets

Add a target with `rustup target add <triple>` and pass `--target <triple>`
to `cargo build`. Useful triples:

- `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl` — fully static
  Linux binaries (what the releases use).
- `x86_64-apple-darwin` + `aarch64-apple-darwin` — on a Mac, build both and
  merge into one universal binary:
  ```
  lipo -create -output xisf2png \
    target/x86_64-apple-darwin/release/xisf2png \
    target/aarch64-apple-darwin/release/xisf2png
  ```

Building macOS binaries requires a Mac; Windows and Linux can be built from
each other with the right toolchain, but the simplest way to get all of them
is the release workflow below.

### Releasing

[`.github/workflows/release.yml`](.github/workflows/release.yml) builds all
four assets above on GitHub's Windows, macOS and Linux runners and attaches
them to a GitHub Release. To cut a release, tag a commit and push the tag:

```
git tag v0.2.0
git push origin v0.2.0
```

To check that everything still builds without publishing anything, run the
workflow manually from the repo's **Actions** tab; the binaries are then
available as workflow artifacts.

## Dependencies

All pure Rust (no C toolchain needed):

| Crate       | Purpose                   |
| ----------- | ------------------------- |
| `roxmltree` | XISF XML header parsing   |
| `base64`    | inline / embedded base64  |
| `hex`       | inline / embedded hex     |
| `flate2`    | zlib decompression        |
| `lz4_flex`  | lz4 / lz4hc decompression |
| `ruzstd`    | zstd decompression        |
| `png`       | PNG encoding              |
