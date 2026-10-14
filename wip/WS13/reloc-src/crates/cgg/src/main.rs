//! `cgg` command-line entry point.
//!
//! A thin shim over [`cgg::analyze`] and [`cgg::emit`]. Three things stay
//! here rather than in the library: the `#[global_allocator]`, since a
//! library should not choose one for whatever links it; `init_tracing`,
//! since installing a process-global subscriber is the application's call;
//! and the exit code, since whether a finding fails a build is policy.

use std::process::ExitCode;

use cgg_format::OutputFormat;
use clap::Parser;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use cgg::cli::Cli;
use cgg::{RunOptions, emit};

/// Extraction is allocation-heavy — a `String` per name, per reference,
/// per qualified path, across every worker at once — and the system
/// allocator serialises under that. Measured on netbox: the same work
/// cost 6.8s of CPU at `--jobs 4` and 10.6s at `--jobs 64`, i.e. 56%
/// more CPU burned to produce identical output, which is why thread
/// scaling stopped paying after four cores.
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let code = e.exit_code();
            e.print().ok();
            return ExitCode::from(code as u8);
        }
    };

    init_tracing(&cli);

    match run(&cli) {
        Ok((code, wall_ms)) => {
            if cli.profile {
                eprint!("{}", cgg_core::profile::render(wall_ms));
            }
            code
        }
        // An error means the analysis was incomplete, so any findings it
        // did produce are untrustworthy. Errors therefore dominate the
        // findings exit code: 1 beats 3.
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Analyze, write everything out, and decide the exit status.
///
/// Returns the wall time alongside it so `main` can render the `--profile`
/// table after every span has dropped, without a process-global.
fn run(cli: &Cli) -> anyhow::Result<(ExitCode, f64)> {
    // The application's startup banner. Here rather than in `analyze`
    // because it names the output format, which never reaches
    // `RunOptions` — `-t` does not change the graph.
    info!(
        version = cgg_core::CGG_VERSION,
        paths = cli.paths.len(),
        format = %OutputFormat::from(cli.format),
        "cgg starting"
    );

    let outcome = cgg::analyze(&RunOptions::from(cli))?;
    let wall_ms = outcome.metrics.wall_ms;
    emit::all(cli, &outcome)?;

    // Exit 3 only when asked for: adding `--dead-code` to an existing
    // pipeline must not break it, and every finding is a hypothesis.
    let code = if cli.fail_on_dead && outcome.dead_code_marked > 0 {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    };
    Ok((code, wall_ms))
}

fn init_tracing(cli: &Cli) {
    let default_level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 => "warn",
            1 => "info",
            _ => "debug",
        }
    };
    let filter = EnvFilter::try_from_env("CGG_LOG")
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}
