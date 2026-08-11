mod changed;
mod cli;
mod commands;
mod error;
mod extensions;
mod install_worker;
mod perf;
mod release;
mod render;

use std::ffi::OsString;
use std::io::Write;

use clap::Parser;

/// Run the Rust CLI with explicit streams.
///
/// Embedders may rely on this function to parse command-line arguments, execute the
/// requested foundation command, and keep machine-readable stdout separate from
/// progress/profile stderr. It does not expose workspace discovery, cache, Lake,
/// or rendering internals as public API.
pub fn run<I, T, O, E>(args: I, stdout: &mut O, stderr: &mut E) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    O: Write + Send,
    E: Write + Send,
{
    // Install the stderr tracing subscriber before anything else so every path
    // — `--version`, `install-worker`, external subcommands — is covered. Quiet
    // by default (warn); `RUST_LOG` opts into detail. Idempotent across calls.
    lean_dup_diagnostics::install_tracing("warn");

    let cli = match cli::Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let code = error.exit_code();
            let rendered = error.to_string();
            let result = if error.use_stderr() {
                write!(stderr, "{rendered}")
            } else {
                write!(stdout, "{rendered}")
            };
            if result.is_err() {
                return 1;
            }
            return code;
        }
    };

    if cli.version {
        return match release::write_version(stdout) {
            Ok(()) => 0,
            Err(error) => {
                let _ = writeln!(stderr, "error: {error}");
                1
            }
        };
    }

    if cli.list {
        return match extensions::write_command_list(stdout) {
            Ok(()) => 0,
            Err(error) => {
                let _ = writeln!(stderr, "error: {error}");
                1
            }
        };
    }

    if let Some(cli::Command::InstallWorker(args)) = cli.command.as_ref() {
        return install_worker::run(args);
    }

    if let Some(cli::Command::External(external)) = cli.command.as_ref() {
        return extensions::run_external(&cli, external, stdout, stderr);
    }

    match commands::run(cli) {
        Ok(outcome) => {
            let exit_code = outcome.exit_code;
            if let Err(error) = render::write_outcome(outcome, stdout, stderr) {
                let _ = writeln!(stderr, "error: {error}");
                1
            } else {
                exit_code
            }
        }
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            1
        }
    }
}
