//! xisf2png - batch convert XISF astronomical images to PNG.

mod pixels;
mod xisf;

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut input_dir: Option<String> = None;
    let mut output_dir: Option<String> = None;
    let mut recursive = false;
    let mut overwrite = false;
    let mut resize4k = false;
    let mut png_only = false;
    let mut font: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--recursive" | "-r" => recursive = true,
            "--overwrite" => overwrite = true,
            "--resize4k" | "-resize4k" => resize4k = true,
            "--png-only" => png_only = true,
            "--font" => {
                i += 1;
                match args.get(i) {
                    Some(f) if !f.is_empty() => font = Some(f.clone()),
                    _ => {
                        eprintln!("--font requires a font name");
                        usage();
                        return ExitCode::from(2);
                    }
                }
            }
            _ if a.starts_with("--font=") => {
                let f = &a["--font=".len()..];
                if f.is_empty() {
                    eprintln!("--font requires a font name");
                    usage();
                    return ExitCode::from(2);
                }
                font = Some(f.to_string());
            }
            "--help" | "-h" | "/?" => {
                usage();
                return ExitCode::SUCCESS;
            }
            _ => {
                if a.starts_with('-') {
                    eprintln!("Unknown option: {a}");
                    usage();
                    return ExitCode::from(2);
                }
                if input_dir.is_none() {
                    input_dir = Some(a.clone());
                } else if output_dir.is_none() {
                    output_dir = Some(a.clone());
                } else {
                    eprintln!("Unexpected argument: {a}");
                    usage();
                    return ExitCode::from(2);
                }
            }
        }
        i += 1;
    }

    // No input folder given: work on the current directory.
    let input_dir = input_dir.unwrap_or_else(|| ".".to_string());
    // --png-only exists purely to run the 4K post-processing on PNGs that
    // already exist, so it implies --resize4k.
    if png_only {
        resize4k = true;
    }

    // Output defaults to the input folder: PNGs land next to their sources
    // (or, with --png-only, are edited in place).
    let output_dir = output_dir.unwrap_or_else(|| input_dir.clone());

    let input_dir = Path::new(&input_dir);
    let output_dir = Path::new(&output_dir);

    if !input_dir.is_dir() {
        eprintln!("Input directory not found: {}", input_dir.display());
        return ExitCode::from(2);
    }

    let ext = if png_only { "png" } else { "xisf" };
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(input_dir, recursive, ext, &mut files);

    files.sort_by(|a, b| {
        a.to_string_lossy()
            .to_lowercase()
            .cmp(&b.to_string_lossy().to_lowercase())
    });

    if files.is_empty() {
        println!("No .{ext} files found.");
        return ExitCode::SUCCESS;
    }

    let (mut converted, mut skipped, mut failed) = (0u32, 0u32, 0u32);
    let mut warnings: Vec<String> = Vec::new();
    let mut magick = Magick::new(font);

    for file in &files {
        // In --png-only mode ImageMagick does all the work, so once it is
        // known to be missing there is nothing useful left to do.
        if png_only && !magick.available() {
            break;
        }

        let rel = file.strip_prefix(input_dir).unwrap_or(file);
        let dest = output_dir.join(rel).with_extension("png");
        let rel_display = rel.display();

        let result = if png_only {
            stage_png(file, &dest, overwrite)
        } else {
            convert_one(file, &dest, overwrite)
        };

        match result {
            Ok(Outcome::Converted) => {
                println!("OK    {rel_display}");
                converted += 1;
                if resize4k && magick.available() {
                    magick.resize_and_label(&dest, &mut warnings);
                }
            }
            Ok(Outcome::Skipped) => {
                println!("SKIP  {rel_display}");
                skipped += 1;
            }
            Err(e) => {
                println!("ERROR {rel_display}: {e}");
                failed += 1;
            }
        }
    }

    println!();
    let verb = if png_only { "Processed" } else { "Converted" };
    println!("{verb}: {converted}   Skipped: {skipped}   Failed: {failed}");

    if !warnings.is_empty() {
        println!();
        for w in &warnings {
            println!("WARNING: {w}");
        }
    }

    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Recursively (or not) gather files with extension `ext` (case-insensitive)
/// under `dir`.
fn collect_files(dir: &Path, recursive: bool, ext: &str, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            if recursive {
                collect_files(&path, recursive, ext, out);
            }
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case(ext))
        {
            out.push(path);
        }
    }
}

enum Outcome {
    Converted,
    Skipped,
}

/// ImageMagick command names to try, in order. ImageMagick 7 installs
/// `magick` (Windows installer, Homebrew, most modern distros); ImageMagick 6,
/// still shipped by Debian/Ubuntu, only has `convert`. The CLI syntax we use
/// is identical on both.
///
/// `convert` is deliberately not tried on Windows: `C:\Windows\System32\convert.exe`
/// is an unrelated filesystem tool that would be picked up instead.
#[cfg(windows)]
const MAGICK_COMMANDS: &[&str] = &["magick"];
#[cfg(not(windows))]
const MAGICK_COMMANDS: &[&str] = &["magick", "convert"];

/// Wraps the ImageMagick post-processing step and remembers which command
/// works so a missing binary is only probed once, not once per file.
struct Magick {
    /// Index into `MAGICK_COMMANDS` of the command to use; `None` once every
    /// candidate has been found missing.
    cmd: Option<usize>,
    /// Font name passed to ImageMagick, or `None` to use its default.
    font: Option<String>,
}

impl Magick {
    fn new(font: Option<String>) -> Self {
        Magick {
            cmd: Some(0),
            font,
        }
    }

    fn available(&self) -> bool {
        self.cmd.is_some()
    }

    /// Post-process a PNG in place: scale (up or down, aspect ratio kept) to
    /// cover 3840x2160, center-crop the overflow so the result is exactly
    /// 3840x2160, and stamp the file name in the bottom-right corner.
    /// Failures are collected as warnings so the batch keeps going; if no
    /// ImageMagick binary can be found, further attempts are disabled.
    fn resize_and_label(&mut self, dest: &Path, warnings: &mut Vec<String>) {
        let label = dest
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        loop {
            let Some(idx) = self.cmd else { return };
            let program = MAGICK_COMMANDS[idx];

            let mut command = std::process::Command::new(program);
            command
                .arg(dest)
                .args(["-resize", "3840x2160^"])
                .args(["-gravity", "center"])
                .args(["-extent", "3840x2160"]);
            if let Some(font) = &self.font {
                command.args(["-font", font]);
            }
            command
                .args(["-pointsize", "48"])
                .args(["-gravity", "southeast"])
                .args(["-fill", "white"])
                .args(["-annotate", "+60+120"])
                .arg(&label)
                .arg(dest);

            match command.output() {
                Ok(out) if out.status.success() => {
                    // ImageMagick still exits 0 on a non-fatal problem such as
                    // an unknown --font (it silently falls back to its default),
                    // reporting it only on stderr. Surface it once, not per file.
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    if let Some(line) = stderr.trim().lines().next() {
                        let w = format!("ImageMagick: {line}");
                        if !warnings.contains(&w) {
                            warnings.push(w);
                        }
                    }
                    return;
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    warnings.push(format!(
                        "ImageMagick failed on {}: {}",
                        dest.display(),
                        stderr.trim().lines().next().unwrap_or("unknown error")
                    ));
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Try the next candidate command; give up when none are left.
                    let next = idx + 1;
                    if next < MAGICK_COMMANDS.len() {
                        self.cmd = Some(next);
                        continue;
                    }
                    self.cmd = None;
                    warnings.push(format!(
                        "ImageMagick not found on PATH (tried {}) - PNGs were not \
                         resized/labelled.",
                        MAGICK_COMMANDS.join(", ")
                    ));
                    return;
                }
                Err(e) => {
                    warnings.push(format!(
                        "Could not run ImageMagick on {}: {e}",
                        dest.display()
                    ));
                    return;
                }
            }
        }
    }
}

fn convert_one(
    src: &Path,
    dest: &Path,
    overwrite: bool,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    if dest.exists() && !overwrite {
        return Ok(Outcome::Skipped);
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = xisf::read(src)?;
    let image = pixels::to_image(&data)?;

    let file = File::create(dest)?;
    let w = BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, image.width, image.height);
    encoder.set_color(if image.channels == 1 {
        png::ColorType::Grayscale
    } else {
        png::ColorType::Rgb
    });
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.pixels)?;
    writer.finish()?;

    Ok(Outcome::Converted)
}

/// `--png-only`: get an existing PNG into place at `dest` so the 4K
/// post-processing can run on it. If `dest` is the source itself (no separate
/// output dir) the file is edited in place; otherwise it is copied first,
/// honouring `--overwrite` like a normal conversion would.
fn stage_png(
    src: &Path,
    dest: &Path,
    overwrite: bool,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    if is_same_file(src, dest) {
        return Ok(Outcome::Converted);
    }
    if dest.exists() && !overwrite {
        return Ok(Outcome::Skipped);
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dest)?;
    Ok(Outcome::Converted)
}

/// True when both paths refer to the same existing file (handles differences
/// like `.` vs `./`, trailing separators, or drive-letter case).
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn usage() {
    eprintln!(
        "xisf2png - batch convert XISF astronomical images to PNG\n\n\
         Usage:\n\
         \x20 xisf2png [input_dir] [output_dir] [--recursive|-r] [--overwrite] [--resize4k]\n\
         \x20 xisf2png [input_dir] [output_dir] --png-only [--recursive|-r] [--overwrite]\n\n\
         If input_dir is omitted, the current folder is used.\n\
         If output_dir is omitted, PNGs are written next to their source files.\n\n\
         Options:\n\
         \x20 -r, --recursive   recurse into subfolders (output mirrors structure)\n\
         \x20     --overwrite    overwrite existing .png files (default: skip)\n\
         \x20     --resize4k     scale each PNG to exactly 3840x2160 (aspect kept,\n\
         \x20                    center-cropped, no padding) and stamp the file\n\
         \x20                    name bottom-right (requires ImageMagick on PATH:\n\
         \x20                    'magick' or, for ImageMagick 6, 'convert')\n\
         \x20     --png-only     skip XISF conversion: take existing .png files in\n\
         \x20                    input_dir and only resize/annotate them (implies\n\
         \x20                    --resize4k). Edited in place when output_dir is\n\
         \x20                    omitted, otherwise copied there first.\n\
         \x20     --font <name>  font for the file-name stamp, as ImageMagick knows\n\
         \x20                    it (see 'magick -list font'). Default: ImageMagick's\n\
         \x20                    standard font. Also accepts --font=<name>.\n\
         \x20 -h, --help        show this help"
    );
}
