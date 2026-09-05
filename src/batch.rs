//! The batch job itself: find files, convert / post-process each one, report
//! progress. Used by both the CLI and the GUI.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use image::{DynamicImage, GrayImage, ImageFormat, RgbImage};

use crate::pixels;
use crate::post::Stamper;
use crate::xisf;

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Folder to scan. Defaults to the current directory when the CLI gets no
    /// positional argument.
    pub input_dir: PathBuf,
    /// Where to write PNGs; `None` means next to the sources (or, with
    /// `png_only`, edit them in place).
    pub output_dir: Option<PathBuf>,
    pub recursive: bool,
    pub overwrite: bool,
    /// Scale to exactly 3840x2160 and stamp the file name.
    pub resize4k: bool,
    /// Skip XISF conversion: operate on existing `.png` files. Implies
    /// `resize4k`.
    pub png_only: bool,
    /// A TrueType / OpenType font file for the stamp; `None` = bundled font.
    pub font: Option<PathBuf>,
}

impl Options {
    /// Effective output directory.
    pub fn output_dir(&self) -> &Path {
        self.output_dir.as_deref().unwrap_or(&self.input_dir)
    }

    /// File extension that will be scanned for.
    pub fn input_ext(&self) -> &'static str {
        if self.png_only {
            "png"
        } else {
            "xisf"
        }
    }

    /// `png_only` is pointless without the 4K step, so it implies it.
    pub fn resize4k(&self) -> bool {
        self.resize4k || self.png_only
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Ok,
    Skipped,
    Failed(String),
}

/// One progress report, sent after each file has been handled.
#[derive(Debug, Clone)]
pub struct Progress {
    /// 1-based index of the file just handled.
    pub index: usize,
    pub total: usize,
    /// Path relative to the input directory.
    pub rel: PathBuf,
    pub status: FileStatus,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub total: usize,
    pub converted: u32,
    pub skipped: u32,
    pub failed: u32,
    /// True when the run was stopped early via the cancel flag.
    pub cancelled: bool,
}

/// Run the whole batch. `report` is called once per file; set `cancel` from
/// another thread to stop after the current file. Errors returned here are
/// fatal setup problems (bad input dir, unreadable font); per-file problems are
/// reported through `FileStatus::Failed` and counted in the summary.
pub fn run(
    opts: &Options,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(&Progress),
) -> Result<Summary, String> {
    if !opts.input_dir.is_dir() {
        return Err(format!(
            "Input directory not found: {}",
            opts.input_dir.display()
        ));
    }

    let stamper = if opts.resize4k() {
        Some(match &opts.font {
            Some(path) => Stamper::from_file(path)?,
            None => Stamper::bundled(),
        })
    } else {
        None
    };

    let mut files = Vec::new();
    collect_files(&opts.input_dir, opts.recursive, opts.input_ext(), &mut files);
    files.sort_by_key(|p| p.to_string_lossy().to_lowercase());

    let mut summary = Summary {
        total: files.len(),
        ..Summary::default()
    };
    let output_dir = opts.output_dir();

    for (i, file) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }

        let rel = file.strip_prefix(&opts.input_dir).unwrap_or(file);
        let dest = output_dir.join(rel).with_extension("png");

        let status = match process_one(file, &dest, opts, stamper.as_ref()) {
            Ok(true) => {
                summary.converted += 1;
                FileStatus::Ok
            }
            Ok(false) => {
                summary.skipped += 1;
                FileStatus::Skipped
            }
            Err(e) => {
                summary.failed += 1;
                FileStatus::Failed(e)
            }
        };

        report(&Progress {
            index: i + 1,
            total: files.len(),
            rel: rel.to_path_buf(),
            status,
        });
    }

    Ok(summary)
}

/// Handle one file. Returns `Ok(true)` if written, `Ok(false)` if skipped.
fn process_one(
    src: &Path,
    dest: &Path,
    opts: &Options,
    stamper: Option<&Stamper>,
) -> Result<bool, String> {
    let in_place = opts.png_only && is_same_file(src, dest);
    if !in_place && dest.exists() && !opts.overwrite {
        return Ok(false);
    }

    let mut img = if opts.png_only {
        image::ImageReader::open(src)
            .and_then(|r| r.decode().map_err(std::io::Error::other))
            .map_err(|e| format!("cannot read PNG: {e}"))?
    } else {
        let data = xisf::read(src).map_err(|e| e.to_string())?;
        let image8 = pixels::to_image(&data).map_err(|e| e.to_string())?;
        image8_to_dynamic(image8)?
    };

    if let Some(stamper) = stamper {
        let label = dest
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        img = stamper.resize_and_label(img, &label);
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create output folder: {e}"))?;
    }
    let file = File::create(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut w = BufWriter::new(file);
    img.write_to(&mut w, ImageFormat::Png)
        .map_err(|e| format!("PNG encoding failed: {e}"))?;

    Ok(true)
}

fn image8_to_dynamic(img: pixels::Image8) -> Result<DynamicImage, String> {
    let (w, h) = (img.width, img.height);
    match img.channels {
        1 => GrayImage::from_raw(w, h, img.pixels)
            .map(DynamicImage::ImageLuma8)
            .ok_or_else(|| "pixel buffer size mismatch".to_string()),
        3 => RgbImage::from_raw(w, h, img.pixels)
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(|| "pixel buffer size mismatch".to_string()),
        n => Err(format!("unsupported channel count {n}")),
    }
}

/// Recursively (or not) gather files with extension `ext` (case-insensitive)
/// under `dir`.
pub fn collect_files(dir: &Path, recursive: bool, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
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

/// True when both paths refer to the same existing file (handles `.` vs `./`,
/// trailing separators, drive-letter case, ...).
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
