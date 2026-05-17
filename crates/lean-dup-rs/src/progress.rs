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
    started: Instant,
    events: Vec<ProgressEvent>,
    timings: Vec<ProfileTiming>,
}

impl Reporter {
    pub(crate) fn new(progress_enabled: bool, profile_enabled: bool) -> Self {
        Self {
            progress_enabled,
            profile_enabled,
            started: Instant::now(),
            events: Vec::new(),
            timings: Vec::new(),
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
        self.events.push(ProgressEvent {
            phase: phase.into(),
            current,
            total,
            message: message.into(),
            elapsed_ms: self.started.elapsed().as_millis(),
        });
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
}
