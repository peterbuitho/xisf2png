//! The batch job itself: find files, convert / post-process each one, report
//! progress. Used by both the CLI and the GUI.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use image::{DynamicImage, GrayImage, ImageFormat, RgbImage};

use crate::fits;
use crate::lookup::{self, Resolver};
use crate::pixels;
use crate::post::{Label, Stamper};
use crate::xisf;

/// Extensions (lower-case) treated as source images in normal mode.
pub const IMAGE_EXTS: &[&str] = &["xisf", "fits", "fit", "fts"];
const FITS_EXTS: &[&str] = &["fits", "fit", "fts"];

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
    /// Look the object up online (CDS Sesame / SIMBAD) and stamp its proper
    /// name and catalogue info instead of the bare file name. Only relevant
    /// when stamping; the file name is the fallback.
    pub lookup: bool,
}

impl Options {
    /// Effective output directory.
    pub fn output_dir(&self) -> &Path {
        self.output_dir.as_deref().unwrap_or(&self.input_dir)
    }

    /// File extensions that will be scanned for.
    pub fn input_exts(&self) -> &'static [&'static str] {
        if self.png_only {
            &["png"]
        } else {
            IMAGE_EXTS
        }
    }

    /// Human-readable description of the input file kind, for messages such
    /// as "No .xisf / .fits files found."
    pub fn input_kind(&self) -> &'static str {
        if self.png_only {
            ".png"
        } else {
            ".xisf / .fits"
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
    /// Title that was stamped on the image, when it differs from the file
    /// name (i.e. the object was identified online).
    pub label: Option<String>,
    /// Something the user should know about this file (e.g. header and file
    /// name disagree about the object).
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub total: usize,
    pub converted: u32,
    pub skipped: u32,
    pub failed: u32,
    /// True when the run was stopped early via the cancel flag.
    pub cancelled: bool,
    /// Run-level warnings (e.g. the online lookup was unreachable).
    pub warnings: Vec<String>,
}

/// Result of handling one file.
struct Outcome {
    written: bool,
    label: Option<String>,
    note: Option<String>,
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
    collect_files(&opts.input_dir, opts.recursive, opts.input_exts(), &mut files);
    files.sort_by_key(|p| p.to_string_lossy().to_lowercase());

    let mut summary = Summary {
        total: files.len(),
        ..Summary::default()
    };
    let output_dir = opts.output_dir();
    let mut resolver = Resolver::new(opts.lookup && stamper.is_some());

    for (i, file) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }

        let rel = file.strip_prefix(&opts.input_dir).unwrap_or(file);
        let dest = output_dir.join(rel).with_extension("png");

        let (status, label, note) =
            match process_one(file, &dest, opts, stamper.as_ref(), &mut resolver) {
                Ok(Outcome {
                    written: true,
                    label,
                    note,
                }) => {
                    summary.converted += 1;
                    (FileStatus::Ok, label, note)
                }
                Ok(Outcome { note, .. }) => {
                    summary.skipped += 1;
                    (FileStatus::Skipped, None, note)
                }
                Err(e) => {
                    summary.failed += 1;
                    (FileStatus::Failed(e), None, None)
                }
            };

        report(&Progress {
            index: i + 1,
            total: files.len(),
            rel: rel.to_path_buf(),
            status,
            label,
            note,
        });
    }

    if let Some(e) = resolver.failure.take() {
        summary.warnings.push(format!(
            "Online object lookup unavailable ({e}); file names were stamped instead."
        ));
    }

    Ok(summary)
}

/// Handle one file.
fn process_one(
    src: &Path,
    dest: &Path,
    opts: &Options,
    stamper: Option<&Stamper>,
    resolver: &mut Resolver,
) -> Result<Outcome, String> {
    let in_place = opts.png_only && is_same_file(src, dest);
    if !in_place && dest.exists() && !opts.overwrite {
        return Ok(Outcome {
            written: false,
            label: None,
            note: None,
        });
    }

    let mut header_object: Option<String> = None;
    let mut img = if opts.png_only {
        image::ImageReader::open(src)
            .and_then(|r| r.decode().map_err(std::io::Error::other))
            .map_err(|e| format!("cannot read PNG: {e}"))?
    } else {
        let data = if has_ext(src, FITS_EXTS) {
            fits::read(src)
        } else {
            xisf::read(src)
        }
        .map_err(|e| e.to_string())?;
        header_object = data.object.clone();
        let image8 = pixels::to_image(&data).map_err(|e| e.to_string())?;
        image8_to_dynamic(image8)?
    };

    let mut label = None;
    let mut note = None;
    if let Some(stamper) = stamper {
        let stem = dest
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let id = lookup::identify(resolver, header_object.as_deref(), &stem);
        if id.label != Label::plain(&stem) {
            label = Some(id.label.title.clone());
        }
        note = id.note;
        img = stamper.resize_and_label(img, &id.label);
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create output folder: {e}"))?;
    }
    let file = File::create(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut w = BufWriter::new(file);
    img.write_to(&mut w, ImageFormat::Png)
        .map_err(|e| format!("PNG encoding failed: {e}"))?;

    Ok(Outcome {
        written: true,
        label,
        note,
    })
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

/// Recursively (or not) gather files whose extension (case-insensitive) is
/// one of `exts`, under `dir`.
pub fn collect_files(dir: &Path, recursive: bool, exts: &[&str], out: &mut Vec<PathBuf>) {
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
                collect_files(&path, recursive, exts, out);
            }
        } else if file_type.is_file() && has_ext(&path, exts) {
            out.push(path);
        }
    }
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| exts.iter().any(|e| s.eq_ignore_ascii_case(e)))
}

/// True when both paths refer to the same existing file (handles `.` vs `./`,
/// trailing separators, drive-letter case, ...).
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
