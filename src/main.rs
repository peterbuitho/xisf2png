//! xisf2png command-line interface.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use xisf2png::{FileStatus, Options};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut opts = Options::default();
    let mut input_dir: Option<String> = None;
    let mut output_dir: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--recursive" | "-r" => opts.recursive = true,
            "--overwrite" => opts.overwrite = true,
            "--resize4k" | "-resize4k" => opts.resize4k = true,
            "--png-only" => opts.png_only = true,
            "--font" => {
                i += 1;
                match args.get(i) {
                    Some(f) if !f.is_empty() => opts.font = Some(PathBuf::from(f)),
                    _ => {
                        eprintln!("--font requires a path to a .ttf/.otf file");
                        usage();
                        return ExitCode::from(2);
                    }
                }
            }
            _ if a.starts_with("--font=") => {
                let f = &a["--font=".len()..];
                if f.is_empty() {
                    eprintln!("--font requires a path to a .ttf/.otf file");
                    usage();
                    return ExitCode::from(2);
                }
                opts.font = Some(PathBuf::from(f));
            }
            "--version" | "-V" => {
                println!("xisf2png {}", xisf2png::VERSION);
                return ExitCode::SUCCESS;
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
    opts.input_dir = PathBuf::from(input_dir.unwrap_or_else(|| ".".to_string()));
    opts.output_dir = output_dir.map(PathBuf::from);

    let ext = opts.input_ext();
    let verb = if opts.png_only { "Processed" } else { "Converted" };

    let summary = xisf2png::run(&opts, &AtomicBool::new(false), &mut |p| {
        let rel = p.rel.display();
        match &p.status {
            FileStatus::Ok => println!("OK    {rel}"),
            FileStatus::Skipped => println!("SKIP  {rel}"),
            FileStatus::Failed(e) => println!("ERROR {rel}: {e}"),
        }
    });

    let summary = match summary {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    if summary.total == 0 {
        println!("No .{ext} files found.");
        return ExitCode::SUCCESS;
    }

    println!();
    println!(
        "{verb}: {}   Skipped: {}   Failed: {}",
        summary.converted, summary.skipped, summary.failed
    );

    if summary.failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn usage() {
    eprintln!(
        "xisf2png {} - batch convert XISF astronomical images to PNG\n\n\
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
         \x20                    name bottom-right\n\
         \x20     --png-only     skip XISF conversion: take existing .png files in\n\
         \x20                    input_dir and only resize/annotate them (implies\n\
         \x20                    --resize4k). Edited in place when output_dir is\n\
         \x20                    omitted, otherwise copied there first.\n\
         \x20     --font <file>  .ttf/.otf font file for the file-name stamp\n\
         \x20                    (default: bundled DejaVu Sans Condensed Bold).\n\
         \x20                    Also accepts --font=<file>.\n\
         \x20 -V, --version     print version\n\
         \x20 -h, --help        show this help",
        xisf2png::VERSION
    );
}
