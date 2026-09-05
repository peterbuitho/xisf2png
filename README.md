# xisf2png

Batch-convert PixInsight **XISF** astronomical images to **PNG**. Written in
Rust; builds to a single self-contained executable.

Each image's full data range is linearly scaled to 0–255 (a plain min/max
stretch — no STF/MTF astronomical stretch). Only the first `<Image>` in a file
is converted. Mono and RGB images are supported.

## Usage

```
xisf2png <input_dir> [output_dir] [--recursive|-r] [--overwrite] [--resize4k]
xisf2png <input_dir> [output_dir] --png-only [--recursive|-r] [--overwrite]
```

If `output_dir` is omitted, PNGs are written next to their source files (or,
with `--png-only`, edited in place).

| Option              | Meaning                                                     |
| ------------------- | ----------------------------------------------------------- |
| `-r`, `--recursive` | Recurse into subfolders; the output tree mirrors the input. |
| `--overwrite`       | Overwrite existing `.png` files (default: skip them).       |
| `--resize4k`        | Scale each PNG (up or down, aspect ratio kept) to cover 3840×2160, then center-crop to exactly 3840×2160 — no padding — and stamp the file name in the bottom-right corner (Franklin Gothic Medium Cond, 48 pt, white). Requires [ImageMagick](https://imagemagick.org) (`magick`) on `PATH`; if it's missing or fails, conversion continues and a warning is printed at the end. |
| `--png-only`        | Skip XISF conversion entirely: pick up existing `.png` files in `input_dir` and only run the `--resize4k` step on them (implies `--resize4k`). With no `output_dir` the PNGs are modified in place; with one they are copied there first, honouring `--overwrite`. |
| `-h`, `--help`      | Show help.                                                  |

Per-file output lines: `OK <path>`, `SKIP <path>`, `ERROR <path>: <reason>`.
A bad file is reported and the batch continues. Exit code is `1` if any file
failed, `2` for a usage error, `0` otherwise.

Examples:

```
xisf2png "D:\captures" "D:\previews" --recursive
xisf2png "D:\previews" --png-only --recursive
```

## Format support

- Sample formats: `UInt8/16/32/64`, `Float32/64` (not `Complex`).
- Pixel storage: planar and interleaved (`Normal`); little- and big-endian.
- Data location: `attachment`, `embedded`, `inline` (base64 / hex).
- Compression: `zlib`, `lz4`, `lz4hc`, `zstd`, with or without byte-shuffle
  (`+sh`).

## Build

Requires a Rust toolchain (install via [rustup](https://rustup.rs)).

```
cargo build --release
```

The binary is `target\release\xisf2png.exe` — a single self-contained
executable, no runtime needed.

On Windows without the MSVC Build Tools, use the GNU toolchain instead:

```
rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release
```

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
