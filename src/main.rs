//! xisf2png command-line interface.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use xisf2png::{FileStatus, Options};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut opts = Options {
        lookup: true,
        ..Options::default()
    };
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
            "--filename" | "--no-lookup" | "--offline" => opts.lookup = false,
            "-j" | "--concurrency" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    Some(n) if n >= 1 => opts.concurrency = n,
                    _ => {
                        eprintln!("-j requires a positive integer");
                        usage();
                        return ExitCode::from(2);
                    }
                }
            }
            _ if a.starts_with("-j") => match a[2..].parse::<usize>() {
                Ok(n) if n >= 1 => opts.concurrency = n,
                _ => {
                    eprintln!("-j requires a positive integer");
                    usage();
                    return ExitCode::from(2);
                }
            },
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
                // Positional: existing files are inputs to process; anything
                // else is a folder (input first, then output).
                if Path::new(a).is_file() {
                    opts.files.push(PathBuf::from(a));
                } else if input_dir.is_none() && opts.files.is_empty() {
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

    // "xisf2png out_dir a.xisf": with explicit files the only folder that
    // makes sense is the output folder.
    if !opts.files.is_empty() && output_dir.is_none() {
        output_dir = input_dir.take();
    }
    // No input folder given: work on the current directory.
    opts.input_dir = PathBuf::from(input_dir.unwrap_or_else(|| ".".to_string()));
    opts.output_dir = output_dir.map(PathBuf::from);

    let kind = opts.input_kind();
    let verb = if opts.png_only { "Processed" } else { "Converted" };

    let summary = xisf2png::run(&opts, &AtomicBool::new(false), &mut |p| {
        let rel = p.rel.display();
        match &p.status {
            FileStatus::Ok => match &p.label {
                Some(label) => println!("OK    {rel}  ->  {label}"),
                None => println!("OK    {rel}"),
            },
            FileStatus::Skipped => println!("SKIP  {rel}"),
            FileStatus::Failed(e) => println!("ERROR {rel}: {e}"),
        }
        if let Some(note) = &p.note {
            println!("      note: {note}");
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
        println!("No {kind} files found.");
        return ExitCode::SUCCESS;
    }

    println!();
    println!(
        "{verb}: {}   Skipped: {}   Failed: {}",
        summary.converted, summary.skipped, summary.failed
    );
    for w in &summary.warnings {
        println!("WARNING: {w}");
    }

    if summary.failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn usage() {
    eprintln!(
        "xisf2png {} - batch convert XISF and FITS astronomical images to PNG\n\n\
         Usage:\n\
         \x20 xisf2png [input_dir] [output_dir] [--recursive|-r] [--overwrite] [--resize4k] [--filename] [-j N]\n\
         \x20 xisf2png [input_dir] [output_dir] --png-only [--recursive|-r] [--overwrite] [--filename]\n\
         \x20 xisf2png <file>... [output_dir] [--overwrite] [--resize4k] [--filename]\n\n\
         Converts every .xisf, .fits, .fit and .fts file found in input_dir, or\n\
         exactly the files given (.png files are only resized/stamped).\n\
         If input_dir is omitted, the current folder is used.\n\
         If output_dir is omitted, PNGs are written next to their source files.\n\n\
         Options:\n\
         \x20 -r, --recursive   recurse into subfolders (output mirrors structure)\n\
         \x20     --overwrite    overwrite existing .png files (default: skip)\n\
         \x20     --resize4k     scale each PNG to exactly 3840x2160 (aspect kept,\n\
         \x20                    center-cropped, no padding) and stamp the object\n\
         \x20                    name bottom-right. The object is identified from\n\
         \x20                    the header OBJECT keyword and/or a catalogue id in\n\
         \x20                    the file name (M31, NGC_7000, Sh2-155, ...), looked\n\
         \x20                    up via CDS Sesame/SIMBAD, and stamped as e.g.\n\
         \x20                    \"Andromeda Galaxy (M 31)\" plus NGC/IC ids, type\n\
         \x20                    and coordinates. Falls back to the file name.\n\
         \x20     --filename     stamp the plain file name: no header OBJECT, no\n\
         \x20                    online lookup (aliases: --no-lookup, --offline)\n\
         \x20     --png-only     skip XISF/FITS conversion: take existing .png files in\n\
         \x20                    input_dir and only resize/annotate them (implies\n\
         \x20                    --resize4k). Edited in place when output_dir is\n\
         \x20                    omitted, otherwise copied there first.\n\
         \x20     --font <file>  .ttf/.otf font file for the file-name stamp\n\
         \x20                    (default: bundled DejaVu Sans Condensed Bold).\n\
         \x20                    Also accepts --font=<file>.\n\
         \x20 -j, --concurrency N  convert N files in parallel\n\
         \x20                    (default: number of CPUs, capped at 8)\n\
         \x20 -V, --version     print version\n\
         \x20 -h, --help        show this help",
        xisf2png::VERSION
    );
}
