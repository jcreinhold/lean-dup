use std::io::{IsTerminal, Write, stderr};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde::Serialize;

/// The process-wide `MultiProgress` that owns every live bar.
///
/// Routing all bars through one `MultiProgress` is what lets the tracing log
/// writer suspend them cleanly (see [`crate::logging`]): a shared handle draws
/// to stderr at a bounded rate, and `suspend` clears the bars while a log line
/// prints, then redraws them below it. The draw target is chosen once from the
/// stderr TTY state — hidden off a terminal, where a per-phase summary line is
/// printed instead.
pub(crate) fn global_multi() -> MultiProgress {
    static MULTI: OnceLock<MultiProgress> = OnceLock::new();
    MULTI
        .get_or_init(|| {
            let target = if stderr().is_terminal() {
                ProgressDrawTarget::stderr_with_hz(8)
            } else {
                ProgressDrawTarget::hidden()
            };
            MultiProgress::with_draw_target(target)
        })
        .clone()
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub phase: String,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: String,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileTiming {
    pub phase: String,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
pub struct Reporter {
    progress_enabled: bool,
    profile_enabled: bool,
    live_progress: bool,
    tty: bool,
    started: Instant,
    events: Vec<ProgressEvent>,
    timings: Vec<ProfileTiming>,
    live: Option<LivePhase>,
}

/// Indicatif bar for the phase currently being reported, plus the friendly
/// key it was started under. Bars are scoped to a phase: when the key
/// changes we finish the old bar and spawn a fresh one.
#[derive(Debug)]
struct LivePhase {
    bar: ProgressBar,
    key: &'static str,
}

impl Reporter {
    pub fn new(progress_enabled: bool, profile_enabled: bool) -> Self {
        Self {
            progress_enabled,
            profile_enabled,
            live_progress: false,
            tty: stderr().is_terminal(),
            started: Instant::now(),
            events: Vec::new(),
            timings: Vec::new(),
            live: None,
        }
    }

    pub fn new_live(progress_enabled: bool, profile_enabled: bool) -> Self {
        let mut reporter = Self::new(progress_enabled, profile_enabled);
        reporter.live_progress = true;
        reporter
    }

    pub fn event(
        &mut self,
        phase: impl Into<String>,
        current: Option<u64>,
        total: Option<u64>,
        message: impl Into<String>,
    ) {
        if !self.progress_enabled {
            return;
        }
        let event = ProgressEvent {
            phase: phase.into(),
            current,
            total,
            message: message.into(),
            elapsed_ms: self.started.elapsed().as_millis(),
        };
        if self.live_progress {
            self.render_live_progress(&event);
        } else {
            self.events.push(event);
        }
    }

    pub fn timing(&mut self, phase: impl Into<String>, duration: Duration) {
        if !self.profile_enabled {
            return;
        }
        self.timings.push(ProfileTiming {
            phase: phase.into(),
            elapsed_ms: duration.as_millis(),
        });
    }

    pub fn measure<T, E>(
        &mut self,
        phase: &'static str,
        work: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let started = Instant::now();
        let result = work(self);
        self.timing(phase, started.elapsed());
        result
    }

    pub fn events(&self) -> &[ProgressEvent] {
        &self.events
    }

    pub fn timings(&self) -> &[ProfileTiming] {
        &self.timings
    }

    pub fn finish_live_progress(&mut self) {
        if let Some(phase) = self.live.take() {
            phase.bar.finish_and_clear();
        }
    }

    fn render_live_progress(&mut self, event: &ProgressEvent) {
        let key = progress_key(&event.phase);
        let phase_changed = self.live.as_ref().is_none_or(|live| live.key != key);

        if phase_changed {
            // Close out the previous bar before starting a new one.
            if let Some(prev) = self.live.take() {
                prev.bar.finish_and_clear();
            }
            self.live = Some(LivePhase {
                bar: new_phase_bar(key, event, self.tty),
                key,
            });
            if !self.tty {
                // Non-TTY: print one summary line per phase transition.
                let _ = writeln!(stderr(), "{}", format_progress_event(event));
            }
        }

        let live = self.live.as_ref().expect("just set");
        if let Some(total) = event.total
            && live.bar.length() != Some(total)
        {
            live.bar.set_length(total);
        }
        if let Some(current) = event.current {
            live.bar.set_position(current);
        } else {
            live.bar.tick();
        }
        live.bar.set_message(event.message.clone());

        let finished = matches!((event.current, event.total), (Some(c), Some(t)) if c >= t);
        if finished {
            // Hold the completed state on screen until the next phase replaces it.
            live.bar.finish_with_message(event.message.clone());
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        self.finish_live_progress();
    }
}

fn new_phase_bar(key: &'static str, event: &ProgressEvent, tty: bool) -> ProgressBar {
    let bar = if let Some(total) = event.total {
        ProgressBar::new(total)
    } else {
        ProgressBar::new_spinner()
    };
    // Adding to the shared `MultiProgress` gives the bar its stderr draw target
    // (or a hidden one off a TTY) and makes it suspendable around log lines.
    let bar = global_multi().add(bar);
    bar.set_prefix(key);
    bar.set_style(if event.total.is_some() {
        bar_style()
    } else {
        spinner_style()
    });
    bar.set_message(event.message.clone());
    // A determinate bar redraws as its position advances, but an indeterminate
    // spinner only ticks when the caller reports a new event. A long blocking
    // phase (a heavy Lean `extract`/`features` call) reports nothing until it
    // returns, so without a steady tick the spinner freezes and the run looks
    // hung. On a TTY, animate it so the elapsed timer visibly advances; off a
    // TTY the bar is hidden and a ticker thread would be pure waste.
    if tty && event.total.is_none() {
        bar.enable_steady_tick(Duration::from_millis(120));
    }
    bar
}

fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:24} [{bar:28.cyan/blue}] {percent:>3}% {pos}/{len} {msg} ({elapsed})")
        .expect("bar template is static and valid")
        .progress_chars("█▉▊▋▌▍▎▏ ")
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:24} {spinner:.cyan} {msg} ({elapsed})")
        .expect("spinner template is static and valid")
}

pub fn format_progress_event(event: &ProgressEvent) -> String {
    let count = match (event.current, event.total) {
        (Some(current), Some(total)) => format!(" {current}/{total}"),
        (Some(current), None) => format!(" {current}"),
        _ => String::new(),
    };
    format!(
        "progress.{phase}{count}: {message} ({elapsed_ms}ms)",
        phase = event.phase,
        message = event.message,
        elapsed_ms = event.elapsed_ms
    )
}

fn progress_key(phase: &str) -> &'static str {
    if phase.contains("worker.lean.index.chunk") {
        "mathlib declarations"
    } else if phase.contains("worker.lean.index.enumerate") {
        "declaration enumeration"
    } else if phase.contains("worker.lean.import") {
        "Lean import"
    } else if phase.contains("worker.lean.semantic") {
        "Lean semantics"
    } else if phase.contains("mathlib.resolve") {
        "mathlib resolver"
    } else if phase.contains("index.mathlib") {
        "mathlib index"
    } else if phase.contains("workspace") {
        "workspace"
    } else if phase.contains("cache") {
        "cache"
    } else if phase.contains("index") {
        "index"
    } else {
        "progress"
    }
}

#[cfg(test)]
mod tests {
    use super::Reporter;

    #[test]
    fn records_typed_progress_and_profile_without_io() {
        let mut reporter = Reporter::new(true, true);
        reporter.event("workspace", Some(1), Some(2), "resolved");
        reporter.timing("workspace", std::time::Duration::from_millis(7));

        assert_eq!(reporter.events()[0].phase, "workspace");
        assert_eq!(reporter.events()[0].current, Some(1));
        assert_eq!(reporter.timings()[0].phase, "workspace");
        assert_eq!(reporter.timings()[0].elapsed_ms, 7);
    }

    #[test]
    fn live_progress_does_not_buffer_duplicate_events() {
        let mut reporter = Reporter::new_live(true, false);
        reporter.event("workspace", Some(1), Some(1), "resolved");

        assert!(reporter.events().is_empty());
    }

    #[test]
    fn progress_key_maps_worker_phases_to_friendly_labels() {
        assert_eq!(super::progress_key("worker.lean.index.chunk.7"), "mathlib declarations");
        assert_eq!(
            super::progress_key("worker.lean.semantic.probe.batch"),
            "Lean semantics"
        );
        assert_eq!(super::progress_key("workspace.resolve"), "workspace");
        assert_eq!(super::progress_key("unrelated.phase"), "progress");
    }
}
