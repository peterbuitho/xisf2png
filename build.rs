//! Build script: on Windows, embed an icon and version information into the
//! executables. Besides giving the .exe files a proper icon in Explorer, a
//! version resource is one of the things antivirus heuristics expect from a
//! legitimate program; bare unsigned binaries without one score worse.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(windows)]
    {
        let version = env!("CARGO_PKG_VERSION");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "xisf2png")
            .set(
                "FileDescription",
                "xisf2png - XISF/FITS astrophoto to PNG converter",
            )
            .set("CompanyName", "peterbuitho (open source)")
            .set(
                "LegalCopyright",
                "Copyright (c) 2026 peterbuitho. Open source: https://github.com/peterbuitho/xisf2png",
            )
            .set("ProductVersion", version)
            .set("FileVersion", version)
            .set("Comments", "https://github.com/peterbuitho/xisf2png");
        if let Err(e) = res.compile() {
            // Never fail the build over metadata (e.g. rc.exe missing on a
            // GNU toolchain without windres).
            println!("cargo:warning=Windows resource not embedded: {e}");
        }
    }
}
