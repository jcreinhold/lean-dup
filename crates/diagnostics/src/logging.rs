//! Process-global `tracing` subscriber that coexists with progress bars.
//!
//! The subscriber writes to stderr — never stdout, which stays reserved for the
//! machine-readable report. Its writer suspends the shared progress bars (see
//! [`crate::progress`]) around each formatted event, so log lines and bars share
//! the terminal without corrupting each other.

use std::io::{self, Write};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

use crate::progress::global_multi;

/// Install the process-global `tracing` subscriber.
///
/// `RUST_LOG` selects the filter; when it is unset or unparseable, `default_level`
/// applies (the CLI passes `"warn"`, keeping normal runs quiet). Events are written
/// to stderr with the live progress bars suspended around each line.
///
/// Idempotent: uses `try_init`, so a subscriber installed earlier — by a test
/// harness or an embedder — wins and this call is a no-op. Safe to call more than
/// once and from any entry point.
pub fn install_tracing(default_level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(SuspendMakeWriter)
        .try_init();
}

/// Hands the fmt layer a fresh [`SuspendWriter`] per event.
struct SuspendMakeWriter;

impl<'writer> MakeWriter<'writer> for SuspendMakeWriter {
    type Writer = SuspendWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        SuspendWriter { buffer: Vec::new() }
    }
}

/// Buffers a single formatted event, then emits it under one `suspend` so the
/// bars clear and redraw exactly once per log line rather than per `write` call.
struct SuspendWriter {
    buffer: Vec<u8>,
}

impl Write for SuspendWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let line = std::mem::take(&mut self.buffer);
        global_multi().suspend(|| {
            let mut stderr = io::stderr().lock();
            stderr.write_all(&line)?;
            stderr.flush()
        })
    }
}

impl Drop for SuspendWriter {
    fn drop(&mut self) {
        // The fmt layer drops the writer at the end of each event; flush here so a
        // line is emitted even if the layer never called `flush` explicitly.
        let _ = self.flush();
    }
}
