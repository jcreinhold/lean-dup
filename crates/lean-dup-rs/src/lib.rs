mod cache;
mod cli;
mod commands;
mod error;
mod lake;
mod progress;
mod render;
mod workspace;

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
    O: Write,
    E: Write,
{
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

    match commands::run(cli) {
        Ok(outcome) => {
            if let Err(error) = render::write_outcome(outcome, stdout, stderr) {
                let _ = writeln!(stderr, "error: {error}");
                1
            } else {
                0
            }
        }
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            1
        }
    }
}
