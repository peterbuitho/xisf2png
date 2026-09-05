//! Single-instance handoff. Windows Explorer starts one process per selected
//! file when a right-click verb is used; file managers on Linux may do the
//! same. The first process to bind the local port becomes the window; every
//! later one sends its paths to it and exits, so a multi-selection ends up as
//! one file list in one window.
//!
//! Plain std TCP on localhost, no dependencies. The protocol is a header line
//! followed by one absolute path per line.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

use eframe::egui;

const PORT: u16 = 47811;
const HEADER: &str = "xisf2png-open 1";

/// Try to become the primary instance.
///
/// Returns the listener to keep (the caller starts [`serve`] once the UI
/// exists) when this process is the primary; `None` when another instance
/// already runs and `paths` have been handed to it, in which case the caller
/// should exit.
pub fn claim(paths: &[PathBuf]) -> Option<TcpListener> {
    match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(listener) => Some(listener),
        Err(_) => {
            if forward(paths) {
                None
            } else {
                // Port taken by something that is not us: run standalone.
                None.or_else(|| TcpListener::bind(("127.0.0.1", 0)).ok())
            }
        }
    }
}

/// Send `paths` to the running instance. True on success.
fn forward(paths: &[PathBuf]) -> bool {
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(150));
        }
        let Ok(mut stream) = TcpStream::connect_timeout(
            &([127, 0, 0, 1], PORT).into(),
            Duration::from_millis(500),
        ) else {
            continue;
        };
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let mut msg = format!("{HEADER}\n");
        for p in paths {
            let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            msg.push_str(&abs.to_string_lossy());
            msg.push('\n');
        }
        if stream.write_all(msg.as_bytes()).is_ok() {
            let _ = stream.shutdown(Shutdown::Write);
            // Wait for the primary's one-byte acknowledgement so we do not
            // exit before it has read everything.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut ack = [0u8; 1];
            let _ = stream.read(&mut ack);
            return true;
        }
    }
    false
}

/// Accept handoffs forever, sending each batch of paths down `tx` and asking
/// egui to repaint and focus the window.
pub fn serve(listener: TcpListener, tx: Sender<Vec<PathBuf>>, ctx: egui::Context) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream);
            let mut header = String::new();
            if reader.read_line(&mut header).is_err() || header.trim_end() != HEADER {
                continue;
            }
            let paths: Vec<PathBuf> = reader
                .by_ref()
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .map(PathBuf::from)
                .collect();
            let mut stream = reader.into_inner();
            let _ = stream.write_all(b"k");
            if !paths.is_empty() && tx.send(paths).is_ok() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        }
    });
}

/// Strip the launcher's quoting quirks and keep only paths that exist.
pub fn paths_from_args() -> Vec<PathBuf> {
    std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}
