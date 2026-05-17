use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use super::WorkerError;
use super::protocol::{ProtocolOutput, Request};

#[derive(Debug, Clone)]
pub(super) struct CallControl {
    pub(super) timeout: Duration,
    pub(super) cancelled: Arc<AtomicBool>,
}

pub(super) trait WorkerTransport {
    fn call(&self, request: Request, control: CallControl) -> Result<ProtocolOutput, WorkerError>;
}
