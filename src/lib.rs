//! xisf2png - batch convert XISF astronomical images to PNG, optionally
//! resizing to 4K and stamping the file name. Shared by the `xisf2png` CLI and
//! the `xisf2png-gui` desktop app.

pub mod batch;
pub mod catalog;
pub mod fits;
pub mod lookup;
pub mod pixels;
pub mod post;
pub mod wcs;
pub mod xisf;

pub use batch::{collect_files, run, FileStatus, Options, Progress, Summary};
pub use post::{Label, Stamper};

/// Crate version, for `--version` and window titles.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
