# xisf2png

Batch-convert **XISF** (PixInsight) and **FITS** astronomical images to
**PNG**, optionally resized to 4K with the object's name and catalogue info
stamped in the corner ("Andromeda Galaxy (M 31)" with NGC/IC ids, type and
coordinates underneath, looked up from SIMBAD). Comes as a command-line tool
and a small desktop app, both Rust with no runtime dependencies, for Windows,
macOS (universal Intel + Apple Silicon) and Linux.

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
optionally an output folder and a stamp font, adjust the options (the 4K
resize with object-name stamp and the online lookup are on by default), and
press **Convert**. Progress and a per-file log stream in as the batch runs;
**Cancel** stops after the current file.

## Command line

```
xisf2png [input_dir] [output_dir] [--recursive|-r] [--overwrite] [--resize4k] [--no-lookup] [--font <file>]
xisf2png [input_dir] [output_dir] --png-only [--recursive|-r] [--overwrite] [--no-lookup] [--font <file>]
```

Every `.xisf`, `.fits`, `.fit` and `.fts` file found is converted. If
`input_dir` is omitted, the current folder is used. If `output_dir` is
omitted, PNGs are written next to their source files (or, with `--png-only`,
edited in place).

| Option              | Meaning                                                     |
| ------------------- | ----------------------------------------------------------- |
| `-r`, `--recursive` | Recurse into subfolders; the output tree mirrors the input. |
| `--overwrite`       | Overwrite existing `.png` files (default: skip them).       |
| `--resize4k`        | Scale each PNG (up or down, aspect ratio kept) to cover 3840×2160, then center-crop to exactly 3840×2160 — no padding — and stamp the object name in the bottom-right corner (see [Object names](#object-names)). |
| `--no-lookup`       | Never go online: stamp the plain file name. (`--offline` is an alias.) |
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

## Object names

With `--resize4k` (or `--png-only`) the stamp is a two-line label:

```
                    Andromeda Galaxy (M 31)
NGC 224  ·  UGC 454  ·  Galaxy (active nucleus)  ·  RA 00h 42m 44s  Dec +41° 16′ 08″
```

The object is identified from three sources and cross-checked:

1. the `OBJECT` keyword in the FITS header or XISF header (what your capture
   software was told the target was);
2. a catalogue designation in the file name — `M31`, `NGC_7000`, `IC 1396`,
   `Sh2-155`, `B33`, `LDN1235`, `vdB141`, `Cr399`, `Mel15`, `Ced214`,
   `Arp273`, `UGC`, `PGC`, `HD`, `HIP` are recognised, in any case and with
   `_`, `-` or a space as separator. Filter letters and exposure times such
   as `_B_120s` are not mistaken for catalogue ids;
3. the image coordinates: the plate solution if the file has one (WCS
   keywords `CRVAL1/2`, `CRPIX1/2`, `CD` matrix or `CDELT`), otherwise the
   mount target (`OBJCTRA`/`OBJCTDEC`, `RA`/`DEC`, or XISF
   `Observation:Center:RA/Dec`).

Names are resolved through the [CDS Sesame](https://cds.unistra.fr/cgi-bin/Sesame)
service (SIMBAD), which understands free-form names too (`OBJECT = 'Pleiades'`
gives "Pleiades (M 45)"). The rules:

- Header and file name disagree → the file name wins, with a `note:` line.
- The named object is more than 2° (plus half the field of view) from where
  the frame points → SIMBAD is asked what deep-sky object actually sits at
  the image centre. If a prominent one is there (Messier, NGC, IC, Sharpless,
  … or a common name), it is stamped instead and a note says why; if not,
  the name is kept and the note reports the distance.
- No usable name at all (`Light_0001.fits`) but coordinates present → the
  frame is identified from its coordinates alone, same prominence rule.
- Nothing resolves, or you are offline → the plain file name is stamped as
  before.
- SIMBAD files a cluster and the nebula around it as separate objects
  (NGC 7380 is "an open cluster"; the Wizard Nebula is Sh2-142). When the
  resolved object is a cluster or nebula without a common name, the named
  nebula at the same position is adopted, giving "Wizard Nebula (NGC 7380)".

Each distinct name or position is looked up once per run, so a folder of 300
subs of one target costs one or two requests. `--no-lookup` disables all of
this.

The second line lists up to three other catalogue ids (Messier, NGC, IC,
Sharpless, Barnard, LBN, LDN, vdB, Collinder, Melotte, Cederblad, Arp, UGC,
PGC, HD, HIP in that priority), the SIMBAD object type in plain words, and
J2000 coordinates. Caldwell numbers are not in SIMBAD and are not recognised.

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

Requires a Rust toolchain (install via [rustup](https://rustup.rs)) and a C
compiler (the TLS library used for the SIMBAD lookup has a little C/assembly;
MSVC Build Tools, Xcode command-line tools, or gcc/clang all work). The same
command builds both binaries on Windows, macOS and Linux:

```
cargo build --release
```

Binaries land in `target/release/` as `xisf2png` and `xisf2png-gui`
(`.exe` on Windows). To build just one: `cargo build --release --bin xisf2png`.

On Linux, the GUI needs two small development packages at build time (it
loads the real libraries dynamically at run time), and the static musl CLI
build needs `musl-tools`:

```
sudo apt install libwayland-dev libxkbcommon-dev musl-tools   # Debian / Ubuntu
sudo dnf install wayland-devel libxkbcommon-devel              # Fedora
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
git tag v0.3.0
git push origin v0.3.0
```

To check that everything still builds without publishing anything, run the
workflow manually from the repo's **Actions** tab; the archives are then
available as workflow artifacts.

## Dependencies

| Crate       | Purpose                                     |
| ----------- | ------------------------------------------- |
| `ureq`      | HTTPS client for the SIMBAD lookup (rustls) |
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
