use std::time::{Duration, Instant};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProgressEvent {
    pub(crate) phase: String,
    pub(crate) current: Option<u64>,
    pub(crate) total: Option<u64>,
    pub(crate) message: String,
    pub(crate) elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProfileTiming {
    pub(crate) phase: String,
    pub(crate) elapsed_ms: u128,
}

#[derive(Debug)]
pub(crate) struct Reporter {
    progress_enabled: bool,
    profile_enabled: bool,
    live_progress: bool,
    started: Instant,
    events: Vec<ProgressEvent>,
    timings: Vec<ProfileTiming>,
}

impl Reporter {
    pub(crate) fn new(progress_enabled: bool, profile_enabled: bool) -> Self {
        Self {
            progress_enabled,
            profile_enabled,
            live_progress: false,
            started: Instant::now(),
            events: Vec::new(),
            timings: Vec::new(),
        }
    }

    pub(crate) fn new_live(progress_enabled: bool, profile_enabled: bool) -> Self {
        Self {
            live_progress: true,
            ..Self::new(progress_enabled, profile_enabled)
        }
    }

    pub(crate) fn event(
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
            eprintln!("{}", format_progress_event(&event));
        } else {
            self.events.push(event);
        }
    }

    pub(crate) fn timing(&mut self, phase: impl Into<String>, duration: Duration) {
        if !self.profile_enabled {
            return;
        }
        self.timings.push(ProfileTiming {
            phase: phase.into(),
            elapsed_ms: duration.as_millis(),
        });
    }

    pub(crate) fn measure<T, E>(
        &mut self,
        phase: &'static str,
        work: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let started = Instant::now();
        let result = work(self);
        self.timing(phase, started.elapsed());
        result
    }

    pub(crate) fn events(&self) -> &[ProgressEvent] {
        &self.events
    }

    pub(crate) fn timings(&self) -> &[ProfileTiming] {
        &self.timings
    }
}

pub(crate) fn format_progress_event(event: &ProgressEvent) -> String {
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
}
