use std::io::Write;

use crate::cli::OutputFormat;
use crate::commands::Outcome;
use crate::error::{AppError, Result};
use lean_dup_diagnostics::perf::{self, CostClass};
use lean_dup_diagnostics::progress::{Reporter, format_progress_event};

pub fn write_outcome<O: Write, E: Write>(mut outcome: Outcome, stdout: &mut O, stderr: &mut E) -> Result<()> {
    perf::measure_result(CostClass::Reporting, "report.render", || {
        write_report(&mut outcome.reporter, stderr)?;
        let rendered = match outcome.output_format {
            OutputFormat::Json => serde_json::to_string_pretty(&outcome.report)?,
            OutputFormat::Text => lean_dup_report::render_text_with(&outcome.report, outcome.render_options),
        };
        if let Some(path) = outcome.output_path {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|source| AppError::Io {
                    message: "could not create CLI output directory",
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            std::fs::write(&path, format!("{rendered}\n")).map_err(|source| AppError::Io {
                message: "could not write CLI output file",
                path,
                source,
            })?;
        }
        writeln!(stdout, "{rendered}")?;
        Ok(())
    })
}

fn write_report<E: Write>(reporter: &mut Reporter, stderr: &mut E) -> Result<()> {
    reporter.finish_live_progress();
    for event in reporter.events() {
        writeln!(stderr, "{}", format_progress_event(event))?;
    }
    for timing in reporter.timings() {
        writeln!(
            stderr,
            "profile.{phase}={elapsed_ms}ms",
            phase = timing.phase,
            elapsed_ms = timing.elapsed_ms
        )?;
    }
    Ok(())
}
