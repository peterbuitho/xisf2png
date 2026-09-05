//! xisf2png desktop GUI (egui / eframe). Same engine as the CLI; the batch
//! runs on a worker thread and streams progress back to the window.

// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod instance;
mod shell;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;

use eframe::egui::{self, Color32, RichText};
use xisf2png::{FileStatus, Options, Progress, Summary};

fn main() -> eframe::Result {
    // Scriptable desktop integration (also used by tests and installers).
    if let Some(flag) = std::env::args().nth(1).filter(|a| a.starts_with("--")) {
        let outcome = match flag.as_str() {
            "--install-context-menu" => shell::install(),
            "--uninstall-context-menu" => shell::uninstall(),
            "--context-menu-status" => shell::Outcome {
                ok: true,
                message: if shell::is_installed() { "installed".into() } else { "not installed".into() },
            },
            other => shell::Outcome {
                ok: false,
                message: format!(
                    "unknown option {other}; use --install-context-menu, --uninstall-context-menu or --context-menu-status"
                ),
            },
        };
        println!("{}", outcome.message);
        std::process::exit(if outcome.ok { 0 } else { 1 });
    }

    // Files / folders given on the command line (Explorer "Open with",
    // right-click verb, `xisf2png-gui a.xisf b.fits`, `open --args` ...).
    let initial = instance::paths_from_args();

    // If another window is already open, hand the paths to it and quit.
    let Some(listener) = instance::claim(&initial) else {
        return Ok(());
    };

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon-256.png"))
        .expect("bundled icon is a valid PNG");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("xisf2png {}", xisf2png::VERSION))
            .with_inner_size([760.0, 600.0])
            .with_min_inner_size([560.0, 420.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "xisf2png",
        options,
        Box::new(move |cc| {
            install_fallback_font(&cc.egui_ctx);
            let (tx, rx) = mpsc::channel();
            instance::serve(listener, tx, cc.egui_ctx.clone());
            let mut app = App {
                resize4k: true,
                lookup: true,
                incoming: Some(rx),
                integration_installed: shell::is_installed(),
                ..App::default()
            };
            app.add_paths(initial);
            Ok(Box::new(app))
        }),
    )
}

/// egui's built-in fonts miss glyphs such as "→", "°", "′" and "″" that the
/// object labels and notes use. Append the bundled DejaVu font as a fallback
/// to every font family so they render instead of showing boxes.
fn install_fallback_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu-fallback".to_owned(),
        Arc::new(egui::FontData::from_static(xisf2png::post::bundled_font_bytes())),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("dejavu-fallback".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Fixed width of the label column in the form (wide enough for
/// "Output folder" so all three fields start at the same x).
const LABEL_WIDTH: f32 = 112.0;

/// One form row: "Label  [ text field stretching to fill ]  [Browse…]".
/// Returns true if the Browse button was clicked.
fn path_row(ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str) -> bool {
    let mut browse = false;
    ui.horizontal(|ui| {
        // Reserve the full label column regardless of text width so the
        // three text fields line up, then draw the label inside it.
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(LABEL_WIDTH, ui.spacing().interact_size.y),
            egui::Sense::hover(),
        );
        let mut column = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        column.label(label);
        // Right-to-left: the button takes its natural size at the right edge,
        // the text field gets everything that is left.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            browse = ui.button("Browse…").clicked();
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .desired_width(ui.available_width()),
            );
        });
    });
    browse
}

/// Messages from the worker thread.
enum Msg {
    Progress(Progress),
    Done(Result<Summary, String>),
}

struct Job {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct App {
    // Form
    input_dir: String,
    output_dir: String,
    recursive: bool,
    overwrite: bool,
    resize4k: bool,
    png_only: bool,
    lookup: bool,
    font_file: String,
    /// Explicit files (right-click selection, drag and drop, "Add files…").
    /// When non-empty they are processed instead of scanning `input_dir`.
    files: Vec<PathBuf>,

    /// Paths handed over by later instances (see `instance.rs`).
    incoming: Option<Receiver<Vec<PathBuf>>>,

    // Desktop integration (right-click / Open with)
    integration_installed: bool,
    integration_msg: Option<(bool, String)>,

    // Run state
    job: Option<Job>,
    log: Vec<Progress>,
    done: usize,
    total: usize,
    summary: Option<Summary>,
    error: Option<String>,
}

impl App {
    fn options(&self) -> Options {
        let input = self.input_dir.trim();
        let output = self.output_dir.trim();
        let font = self.font_file.trim();
        Options {
            input_dir: PathBuf::from(if input.is_empty() { "." } else { input }),
            output_dir: (!output.is_empty()).then(|| PathBuf::from(output)),
            recursive: self.recursive,
            overwrite: self.overwrite,
            resize4k: self.resize4k,
            png_only: self.png_only,
            font: (!font.is_empty()).then(|| PathBuf::from(font)),
            lookup: self.lookup,
            files: self.files.clone(),
        }
    }

    /// Take in paths from the command line, another instance, or a drop:
    /// folders become the input folder, files join the file list.
    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if p.is_dir() {
                self.input_dir = p.display().to_string();
            } else if p.is_file() && !self.files.contains(&p) {
                self.files.push(p);
            }
        }
    }

    /// Paths from other instances and from drag-and-drop onto the window.
    fn collect_incoming(&mut self, ctx: &egui::Context) {
        let mut new = Vec::new();
        if let Some(rx) = &self.incoming {
            while let Ok(batch) = rx.try_recv() {
                new.extend(batch);
            }
        }
        ctx.input(|i| {
            new.extend(i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()));
        });
        if !new.is_empty() {
            self.add_paths(new);
        }
    }

    fn pick_files(&mut self) {
        let mut exts: Vec<&str> = shell::EXTENSIONS.to_vec();
        exts.push("png");
        if let Some(picked) = rfd::FileDialog::new()
            .add_filter("Astro images", &exts)
            .add_filter("All files", &["*"])
            .pick_files()
        {
            self.add_paths(picked);
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let opts = self.options();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        self.log.clear();
        self.done = 0;
        self.total = 0;
        self.summary = None;
        self.error = None;

        let ctx = ctx.clone();
        let cancel_flag = Arc::clone(&cancel);
        let handle = std::thread::spawn(move || {
            let result = xisf2png::run(&opts, &cancel_flag, &mut |p| {
                let _ = tx.send(Msg::Progress(p.clone()));
                ctx.request_repaint();
            });
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        });

        self.job = Some(Job {
            rx,
            cancel,
            handle: Some(handle),
        });
    }

    /// Pull everything the worker has sent so far.
    fn poll(&mut self) {
        let Some(job) = &mut self.job else { return };
        let mut finished = false;
        while let Ok(msg) = job.rx.try_recv() {
            match msg {
                Msg::Progress(p) => {
                    self.done = p.index;
                    self.total = p.total;
                    self.log.push(p);
                }
                Msg::Done(Ok(s)) => {
                    self.total = s.total;
                    self.summary = Some(s);
                    finished = true;
                }
                Msg::Done(Err(e)) => {
                    self.error = Some(e);
                    finished = true;
                }
            }
        }
        if finished {
            if let Some(h) = job.handle.take() {
                let _ = h.join();
            }
            self.job = None;
        }
    }

    fn browse_folder(current: &str) -> Option<PathBuf> {
        let mut dlg = rfd::FileDialog::new();
        if !current.trim().is_empty() {
            dlg = dlg.set_directory(current.trim());
        }
        dlg.pick_folder()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        self.collect_incoming(ui.ctx());
        let running = self.job.is_some();
        let file_mode = !self.files.is_empty();

        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.heading("XISF / FITS → PNG batch converter");
            ui.add_space(6.0);

            // ---- Inputs ---------------------------------------------------
            ui.add_enabled_ui(!running, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;

                if file_mode {
                    // Explicit file list replaces the input folder.
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(LABEL_WIDTH, ui.spacing().interact_size.y),
                            egui::Sense::hover(),
                        );
                        let mut column = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        column.label("Files");
                        ui.label(format!("{} selected", self.files.len()));
                        if ui.button("Add files…").clicked() {
                            self.pick_files();
                        }
                        if ui.button("Clear").clicked() {
                            self.files.clear();
                        }
                    });
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("files")
                            .max_height(110.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                for f in &self.files {
                                    let name =
                                        f.file_name().map(|n| n.to_string_lossy().into_owned());
                                    ui.label(
                                        RichText::new(name.unwrap_or_default())
                                            .text_style(egui::TextStyle::Monospace),
                                    )
                                    .on_hover_text(f.display().to_string());
                                }
                            });
                    });
                } else {
                    if path_row(ui, "Input folder", &mut self.input_dir, "current folder") {
                        if let Some(p) = Self::browse_folder(&self.input_dir) {
                            self.input_dir = p.display().to_string();
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
                        if ui.button("Add files…").clicked() {
                            self.pick_files();
                        }
                        ui.label(
                            RichText::new("or drop files / a folder onto this window")
                                .weak()
                                .small(),
                        );
                    });
                }

                let output_hint = if file_mode {
                    "next to each source file"
                } else if self.png_only {
                    "same as input (edit PNGs in place)"
                } else {
                    "same as input (PNGs next to sources)"
                };
                if path_row(ui, "Output folder", &mut self.output_dir, output_hint) {
                    let start = if self.output_dir.trim().is_empty() {
                        self.input_dir.clone()
                    } else {
                        self.output_dir.clone()
                    };
                    if let Some(p) = Self::browse_folder(&start) {
                        self.output_dir = p.display().to_string();
                    }
                }

                if path_row(
                    ui,
                    "Stamp font",
                    &mut self.font_file,
                    "bundled DejaVu Sans Condensed Bold",
                ) {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Fonts", &["ttf", "otf"])
                        .pick_file()
                    {
                        self.font_file = p.display().to_string();
                    }
                }

                ui.add_space(6.0);

                // ---- Options ----------------------------------------------
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled(
                        !file_mode,
                        egui::Checkbox::new(&mut self.recursive, "Recurse into subfolders"),
                    );
                    ui.checkbox(&mut self.overwrite, "Overwrite existing PNGs");
                    ui.add_enabled(
                        !file_mode,
                        egui::Checkbox::new(&mut self.png_only, "PNG only (no XISF/FITS conversion)"),
                    )
                    .on_hover_text(
                        "Take existing .png files and only resize/stamp them. \
                         Implies 4K resize. (With an explicit file list, each file \
                         is handled by its extension.)",
                    );
                    ui.add_enabled(
                        !self.png_only,
                        egui::Checkbox::new(
                            &mut self.resize4k,
                            "Resize to 3840×2160 and stamp object name",
                        ),
                    );
                    ui.add_enabled(
                        self.resize4k || self.png_only,
                        egui::Checkbox::new(
                            &mut self.lookup,
                            "Stamp object name (SIMBAD lookup); unticked = file name",
                        ),
                    )
                    .on_hover_text(
                        "Ticked: identify the target from the header OBJECT keyword, the \
                         image coordinates and/or a catalogue id in the file name (M31, \
                         NGC_7000, Sh2-155…) and stamp its proper name, other catalogue \
                         ids, type and coordinates; falls back to the file name when \
                         nothing is found or offline.\n\
                         Unticked: stamp the plain file name (same as --filename in the CLI).",
                    );
                });
            });

            // ---- Desktop integration ------------------------------------------
            egui::CollapsingHeader::new("Right-click / \"Open with\" integration")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(RichText::new(shell::HINT).weak());
                    if shell::supported() {
                        ui.horizontal(|ui| {
                            let label = if self.integration_installed {
                                shell::UNINSTALL_LABEL
                            } else {
                                shell::INSTALL_LABEL
                            };
                            if ui.button(label).clicked() {
                                let outcome = if self.integration_installed {
                                    shell::uninstall()
                                } else {
                                    shell::install()
                                };
                                self.integration_installed = shell::is_installed();
                                self.integration_msg = Some((outcome.ok, outcome.message));
                            }
                            if let Some((ok, msg)) = &self.integration_msg {
                                let color = if *ok {
                                    Color32::from_rgb(120, 200, 120)
                                } else {
                                    Color32::from_rgb(230, 120, 120)
                                };
                                ui.label(RichText::new(msg).color(color).small());
                            }
                        });
                    }
                });

            ui.add_space(8.0);

            // ---- Run / Cancel -----------------------------------------------
            ui.horizontal(|ui| {
                if running {
                    if ui.button("Cancel").clicked() {
                        if let Some(job) = &self.job {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                    }
                    ui.spinner();
                    let frac = if self.total > 0 {
                        self.done as f32 / self.total as f32
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .text(format!("{} / {}", self.done, self.total))
                            .desired_width(ui.available_width()),
                    );
                } else {
                    let label = if file_mode {
                        format!("Convert {} file{}", self.files.len(), if self.files.len() == 1 { "" } else { "s" })
                    } else if self.png_only {
                        "Resize && stamp PNGs".to_string()
                    } else {
                        "Convert".to_string()
                    };
                    if ui
                        .add(egui::Button::new(RichText::new(label).strong()))
                        .clicked()
                    {
                        self.start(ui.ctx());
                    }
                    if let Some(s) = &self.summary {
                        let verb = if self.png_only { "Processed" } else { "Converted" };
                        let text = if s.total == 0 {
                            format!("No {} files found.", self.options().input_kind())
                        } else {
                            format!(
                                "{verb}: {}   Skipped: {}   Failed: {}{}",
                                s.converted,
                                s.skipped,
                                s.failed,
                                if s.cancelled { "   (cancelled)" } else { "" }
                            )
                        };
                        let color = if s.failed > 0 {
                            Color32::from_rgb(230, 120, 120)
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.label(RichText::new(text).color(color));
                    }
                    if let Some(e) = &self.error {
                        ui.colored_label(Color32::from_rgb(230, 120, 120), e);
                    }
                }
            });
            if let Some(s) = &self.summary {
                for w in &s.warnings {
                    ui.colored_label(Color32::from_rgb(235, 180, 90), format!("⚠ {w}"));
                }
            }

            ui.add_space(6.0);
            ui.separator();

            // ---- Log --------------------------------------------------------
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let mono = egui::TextStyle::Monospace;
                    for p in &self.log {
                        let (tag, color) = match &p.status {
                            FileStatus::Ok => ("OK   ", Color32::from_rgb(120, 200, 120)),
                            FileStatus::Skipped => ("SKIP ", Color32::GRAY),
                            FileStatus::Failed(_) => ("ERROR", Color32::from_rgb(230, 120, 120)),
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tag).text_style(mono.clone()).color(color));
                            let mut line = p.rel.display().to_string();
                            if let FileStatus::Failed(e) = &p.status {
                                line.push_str(": ");
                                line.push_str(e);
                            }
                            ui.label(RichText::new(line).text_style(mono.clone()));
                            if let Some(label) = &p.label {
                                ui.label(
                                    RichText::new(format!("→ {label}"))
                                        .text_style(mono.clone())
                                        .color(Color32::from_rgb(140, 190, 255)),
                                );
                            }
                        });
                        if let Some(note) = &p.note {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("     ").text_style(mono.clone()));
                                ui.label(
                                    RichText::new(format!("note: {note}"))
                                        .text_style(mono.clone())
                                        .color(Color32::from_rgb(235, 180, 90)),
                                );
                            });
                        }
                    }
                });
        });

        // ---- Drag-and-drop overlay ----------------------------------------------
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let rect = ui.ctx().content_rect();
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop-overlay"),
            ));
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(110));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop to add files (or a folder)",
                egui::FontId::proportional(28.0),
                Color32::WHITE,
            );
        }
    }
}
