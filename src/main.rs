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

    for a in &args {
        match a.as_str() {
            "--recursive" | "-r" => recursive = true,
            "--overwrite" => overwrite = true,
            "--resize4k" | "-resize4k" => resize4k = true,
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
    }

    let Some(input_dir) = input_dir else {
        usage();
        return ExitCode::from(2);
    };
    // Output defaults to the input folder: PNGs land next to their sources.
    let output_dir = output_dir.unwrap_or_else(|| input_dir.clone());

    let input_dir = Path::new(&input_dir);
    let output_dir = Path::new(&output_dir);

    if !input_dir.is_dir() {
        eprintln!("Input directory not found: {}", input_dir.display());
        return ExitCode::from(2);
    }

    let mut files: Vec<PathBuf> = Vec::new();
    collect_xisf(input_dir, recursive, &mut files);

    files.sort_by(|a, b| {
        a.to_string_lossy()
            .to_lowercase()
            .cmp(&b.to_string_lossy().to_lowercase())
    });

    if files.is_empty() {
        println!("No .xisf files found.");
        return ExitCode::SUCCESS;
    }

    let (mut converted, mut skipped, mut failed) = (0u32, 0u32, 0u32);
    let mut warnings: Vec<String> = Vec::new();
    // Set to false after a "program not found" error so we don't retry per file.
    let mut magick_available = true;

    for file in &files {
        let rel = file.strip_prefix(input_dir).unwrap_or(file);
        let dest = output_dir.join(rel).with_extension("png");
        let rel_display = rel.display();

        match convert_one(file, &dest, overwrite) {
            Ok(Outcome::Converted) => {
                println!("OK    {rel_display}");
                converted += 1;
                if resize4k && magick_available {
                    resize_and_label(&dest, &mut warnings, &mut magick_available);
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
    println!("Converted: {converted}   Skipped: {skipped}   Failed: {failed}");

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

/// Recursively (or not) gather `*.xisf` files under `dir`.
fn collect_xisf(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
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
                collect_xisf(&path, recursive, out);
            }
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("xisf"))
        {
            out.push(path);
        }
    }
}

enum Outcome {
    Converted,
    Skipped,
}

/// Post-process a freshly written PNG with ImageMagick: scale (up or down,
/// aspect ratio kept) to cover 3840x2160, center-crop the overflow so the
/// result is exactly 3840x2160, and stamp the file name in the bottom-right
/// corner. Failures are collected as warnings so the batch keeps going; a
/// missing `magick` binary disables further attempts.
fn resize_and_label(dest: &Path, warnings: &mut Vec<String>, magick_available: &mut bool) {
    let label = dest
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let result = std::process::Command::new("magick")
        .arg(dest)
        .args(["-resize", "3840x2160^"])
        .args(["-gravity", "center"])
        .args(["-extent", "3840x2160"])
        .args(["-font", "Franklin-Gothic-Medium-Cond"])
        .args(["-pointsize", "48"])
        .args(["-gravity", "southeast"])
        .args(["-fill", "white"])
        .args(["-annotate", "+30+30"])
        .arg(&label)
        .arg(dest)
        .output();

    match result {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warnings.push(format!(
                "ImageMagick failed on {}: {}",
                dest.display(),
                stderr.trim().lines().next().unwrap_or("unknown error")
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            *magick_available = false;
            warnings.push(
                "ImageMagick ('magick') not found on PATH - PNGs were converted \
                 but not resized/labelled."
                    .into(),
            );
        }
        Err(e) => {
            warnings.push(format!("Could not run ImageMagick on {}: {e}", dest.display()));
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

fn usage() {
    eprintln!(
        "xisf2png - batch convert XISF astronomical images to PNG\n\n\
         Usage:\n\
         \x20 xisf2png <input_dir> [output_dir] [--recursive|-r] [--overwrite] [--resize4k]\n\n\
         If output_dir is omitted, PNGs are written next to their source files.\n\n\
         Options:\n\
         \x20 -r, --recursive   recurse into subfolders (output mirrors structure)\n\
         \x20     --overwrite    overwrite existing .png files (default: skip)\n\
         \x20     --resize4k     scale each PNG to exactly 3840x2160 (aspect kept,\n\
         \x20                    center-cropped, no padding) and stamp the file\n\
         \x20                    name bottom-right (requires ImageMagick on PATH)\n\
         \x20 -h, --help        show this help"
    );
}
