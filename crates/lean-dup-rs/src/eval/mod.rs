pub(crate) mod table;

mod labels;
pub(crate) mod memory;
mod scoring;
mod suites;

pub(crate) use suites::{EvalRequest, EvaluationReport, run};
