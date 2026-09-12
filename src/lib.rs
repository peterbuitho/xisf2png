//! xisf2png - batch convert XISF astronomical images to PNG, optionally
//! resizing to 4K and stamping the file name. Shared by the `xisf2png` CLI and
//! the `xisf2png-gui` desktop app.
//!
//! The conversion pipeline itself lives in the shared
//! [`astropng-core`](https://github.com/peterbuitho/astropng-core) crate,
//! also used by the Go/Nim/Zig/Scala ports of this program (via its C ABI).
//! This crate re-exports the same public surface it always has, backed by
//! that shared implementation.

pub use astropng_core::{collect_files, run, FileStatus, Label, Options, Progress, Stamper, Summary};

/// Kept as `xisf2png::post` for the GUI, which needs the bundled font bytes
/// directly (for egui's font registration) rather than through [`Stamper`].
pub mod post {
    pub use astropng_core::post::bundled_font_bytes;
}

/// Crate version, for `--version` and window titles.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
