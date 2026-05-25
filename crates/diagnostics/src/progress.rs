use std::io::{IsTerminal, Write, stderr};
use std::time::{Duration, Instant};

use serde::Serialize;

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
    live_state: LiveProgressState,
}

#[derive(Debug, Default)]
struct LiveProgressState {
    active: bool,
    last_rendered_at: Option<Instant>,
    last_key: Option<&'static str>,
    last_bucket: Option<u64>,
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
            live_state: LiveProgressState::default(),
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
        if self.live_state.active {
            let _ = writeln!(stderr());
            self.live_state.active = false;
        }
    }

    fn render_live_progress(&mut self, event: &ProgressEvent) {
        let key = progress_key(&event.phase);
        let bucket = progress_bucket(event);
        let now = Instant::now();
        let phase_changed = self.live_state.last_key != Some(key);
        let bucket_changed = self.live_state.last_bucket != bucket;
        let finished = matches!((event.current, event.total), (Some(current), Some(total)) if current >= total);
        let stale = self
            .live_state
            .last_rendered_at
            .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(250));

        if !phase_changed && !finished && (!bucket_changed || !stale) {
            return;
        }

        let mut stderr = stderr();
        if self.tty {
            let _ = write!(stderr, "\r{}", format_progress_bar(event, key));
        } else if phase_changed || finished {
            let _ = writeln!(stderr, "{}", format_progress_event(event));
        } else {
            return;
        }
        let _ = stderr.flush();
        self.live_state.active = self.tty;
        self.live_state.last_rendered_at = Some(now);
        self.live_state.last_key = Some(key);
        self.live_state.last_bucket = bucket;
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        self.finish_live_progress();
    }
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

fn progress_bucket(event: &ProgressEvent) -> Option<u64> {
    let (Some(current), Some(total)) = (event.current, event.total) else {
        return None;
    };
    if total == 0 {
        return Some(0);
    }
    Some(current.saturating_mul(1000) / total)
}

fn format_progress_bar(event: &ProgressEvent, key: &str) -> String {
    let elapsed = format_elapsed(event.elapsed_ms);
    let Some(total) = event.total else {
        return format!(
            "{key:24} [----------------------------]       {} ({elapsed})",
            event.message
        );
    };
    let current = event.current.unwrap_or(0).min(total);
    let width = 28_u64;
    let filled = current.saturating_mul(width).checked_div(total).unwrap_or(width);
    let empty = width.saturating_sub(filled);
    let percent = if total != 0 {
        (current as f64 / total as f64) * 100.0
    } else {
        100.0
    };
    format!(
        "{key:24} [{done}{todo}] {percent:5.1}% {current}/{total} ({elapsed})",
        done = "#".repeat(filled as usize),
        todo = "-".repeat(empty as usize),
    )
}

fn format_elapsed(elapsed_ms: u128) -> String {
    let seconds = elapsed_ms / 1000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes:02}:{seconds:02}")
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
    fn live_progress_formats_as_bar() {
        let event = super::ProgressEvent {
            phase: "worker.lean.index.chunk".to_owned(),
            current: Some(50),
            total: Some(100),
            message: "indexed declarations".to_owned(),
            elapsed_ms: 65_000,
        };

        let rendered = super::format_progress_bar(&event, super::progress_key(&event.phase));

        assert!(rendered.contains("mathlib declarations"));
        assert!(rendered.contains("50.0%"));
        assert!(rendered.contains("50/100"));
        assert!(rendered.contains("01:05"));
    }
}
