//! xisf2png desktop GUI (egui / eframe). Same engine as the CLI; the batch
//! runs on a worker thread and streams progress back to the window.

// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;

use eframe::egui::{self, Color32, RichText};
use xisf2png::{FileStatus, Options, Progress, Summary};

fn main() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png"))
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
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
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
    font_file: String,

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
        let running = self.job.is_some();

        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.heading("XISF → PNG batch converter");
            ui.add_space(6.0);

            // ---- Folders --------------------------------------------------
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("paths")
                    .num_columns(3)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Input folder");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.input_dir)
                                .hint_text("current folder")
                                .desired_width(f32::INFINITY),
                        );
                        if ui.button("Browse…").clicked() {
                            if let Some(p) = Self::browse_folder(&self.input_dir) {
                                self.input_dir = p.display().to_string();
                            }
                        }
                        ui.end_row();

                        ui.label("Output folder");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.output_dir)
                                .hint_text(if self.png_only {
                                    "same as input (edit PNGs in place)"
                                } else {
                                    "same as input (PNGs next to sources)"
                                })
                                .desired_width(f32::INFINITY),
                        );
                        if ui.button("Browse…").clicked() {
                            let start = if self.output_dir.trim().is_empty() {
                                self.input_dir.as_str()
                            } else {
                                self.output_dir.as_str()
                            };
                            if let Some(p) = Self::browse_folder(start) {
                                self.output_dir = p.display().to_string();
                            }
                        }
                        ui.end_row();

                        ui.label("Stamp font");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.font_file)
                                .hint_text("bundled DejaVu Sans Condensed Bold")
                                .desired_width(f32::INFINITY),
                        );
                        if ui.button("Browse…").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Fonts", &["ttf", "otf"])
                                .pick_file()
                            {
                                self.font_file = p.display().to_string();
                            }
                        }
                        ui.end_row();
                    });

                ui.add_space(6.0);

                // ---- Options ----------------------------------------------
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.recursive, "Recurse into subfolders");
                    ui.checkbox(&mut self.overwrite, "Overwrite existing PNGs");
                    ui.checkbox(&mut self.png_only, "PNG only (no XISF conversion)")
                        .on_hover_text(
                            "Take existing .png files and only resize/stamp them. \
                             Implies 4K resize.",
                        );
                    ui.add_enabled(
                        !self.png_only,
                        egui::Checkbox::new(
                            &mut self.resize4k,
                            "Resize to 3840×2160 and stamp file name",
                        ),
                    );
                });
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
                    let label = if self.png_only { "Resize && stamp PNGs" } else { "Convert" };
                    if ui
                        .add(egui::Button::new(RichText::new(label).strong()))
                        .clicked()
                    {
                        self.start(ui.ctx());
                    }
                    if let Some(s) = &self.summary {
                        let verb = if self.png_only { "Processed" } else { "Converted" };
                        let text = if s.total == 0 {
                            format!("No .{} files found.", self.options().input_ext())
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
                        });
                    }
                });
        });
    }
}
