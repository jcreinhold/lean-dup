mod table;

mod labels;
mod memory;
mod scoring;
mod stage_metrics;
mod suites;

pub use memory::peak_rss_bytes;
pub use suites::{EvalRequest, EvaluationReport, run};
pub use table::render_metrics;
