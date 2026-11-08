//! `nanite-viewer-rs` - the Rust replacement for `pbrtexture.exe`.
//!
//! All the logic lives in [`nanite_viewer_rs::app`]; this file is only the process entry point, so
//! the CLI parsing, the frame loop and the camera framing stay unit-testable.
//!
//! ```text
//! nanite-viewer-rs --model bunny --nanite-blob
//! nanite-viewer-rs --model bunny.fcc
//! nanite-viewer-rs --model bunny --capture out.png --captureframes 10 --raster-stats
//! ```
//!
//! The flag set is the pbrtexture one (`pbrtexture.cpp:316-350`) so the existing C++ test command
//! lines work here unchanged.

use std::process::ExitCode;

use nanite_viewer_rs::app::{self, Args};

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("nanite-viewer-rs: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    init_tracing(&args.log);

    if let Err(error) = app::run(args) {
        eprintln!("nanite-viewer-rs: {error:#}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Same levels as `nanite-bake-rs`, so one `--log` spelling covers both Rust tools.
fn init_tracing(level: &str) {
    let filter = match level.to_ascii_lowercase().as_str() {
        "off" => "off",
        "error" => "error",
        "warn" => "warn",
        "info" => "info",
        "debug" => "debug",
        "trace" => "trace",
        other => {
            eprintln!("nanite-viewer-rs: unknown log level '{other}', using info");
            "info"
        }
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_target(false)
        .try_init();
}
