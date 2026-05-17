use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    DeclarationRow, FeatureRow, Fingerprints, ModuleDescriptor, ProbeResult, RoleFeature, SourceSpan, WorkerDiagnostic,
    WorkerError, WorkerEvent, WorkerVersion,
};

pub(super) const SCHEMA_VERSION: &str = "lean-dup.worker.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Command {
    Extract,
    Features,
    Probe,
    Doctor,
    Version,
}

impl Command {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Extract => "extract",
            Self::Features => "features",
            Self::Probe => "probe",
            Self::Doctor => "doctor",
            Self::Version => "version",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResponseKind {
    VersionResult,
    DoctorResult,
    DeclarationRow,
    FeatureRow,
    ProbeResult,
    Progress,
    Complete,
    Error,
}

#[derive(Debug, Clone)]
pub(super) struct Request {
    pub(super) request_id: String,
    pub(super) command: Command,
    payload: Value,
    capabilities: Vec<String>,
}

impl Request {
    pub(super) fn new(request_id: String, command: Command, payload: Value) -> Self {
        Self {
            request_id,
            command,
            payload,
            capabilities: Vec::new(),
        }
    }

    pub(super) fn to_json(&self) -> Value {
        let mut object = match &self.payload {
            Value::Object(object) => object.clone(),
            _ => Map::new(),
        };
        object.insert("schema_version".to_owned(), json!(SCHEMA_VERSION));
        object.insert("request_id".to_owned(), json!(self.request_id));
        object.insert("command".to_owned(), json!(self.command.as_str()));
        if !self.capabilities.is_empty() {
            object.insert("capabilities".to_owned(), json!(self.capabilities));
        }
        Value::Object(object)
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProtocolOutput {
    pub(super) rows: Vec<Row>,
    pub(super) events: Vec<WorkerEvent>,
    pub(super) diagnostics: Vec<WorkerDiagnostic>,
}

#[derive(Debug, Clone)]
pub(super) enum Row {
    Version(WorkerVersion),
    Declaration(DeclarationRow),
    Feature(FeatureRow),
    Probe(ProbeResult),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema_version: String,
    request_id: Option<String>,
    command: Option<Command>,
    kind: ResponseKind,
    payload: Value,
    extensions: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerVersionPayload {
    protocol_version: String,
    worker_version: String,
    lean_version: Option<String>,
    semantic_versions: SemanticVersionsPayload,
    supported_commands: Vec<Command>,
    supported_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticVersionsPayload {
    extract: String,
    features: String,
    probe: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclarationPayload {
    declaration_id: String,
    origin: String,
    module: String,
    qualified_name: String,
    display_name: String,
    kind: String,
    visibility: String,
    modifiers: Vec<String>,
    source_span: Option<SourceSpan>,
    statement_text: String,
    status_flags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FeaturePayload {
    declaration_id: String,
    feature_version: String,
    fingerprints: Fingerprints,
    role_features: Vec<RoleFeature>,
    binder_count: u64,
    low_signal_markers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbePayload {
    pair_id: String,
    left_declaration_id: String,
    right_declaration_id: String,
    status: String,
    same_statement: bool,
    same_up_to_safe_reordering: bool,
    connective_equivalent: bool,
    specializes_left_to_right: bool,
    specializes_right_to_left: bool,
    mutual_implication_shape: bool,
    same_reducible_definition: bool,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressPayload {
    phase: String,
    current: Option<u64>,
    total: Option<u64>,
    module: Option<String>,
    declaration: Option<String>,
    elapsed_ms: Option<u64>,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletePayload {
    row_counts: Map<String, Value>,
    elapsed_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorPayload {
    code: String,
    fatal: bool,
    message: String,
    details: Option<Value>,
}

pub(super) fn modules_payload(workspace_root: &str, modules: &[ModuleDescriptor]) -> Value {
    json!({
        "workspace_root": workspace_root,
        "modules": modules,
    })
}

pub(super) fn parse_output(
    stdout: &str,
    expected_request_id: &str,
    expected_command: Command,
) -> Result<ProtocolOutput, WorkerError> {
    let mut rows = Vec::new();
    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    let mut complete = None;

    for (index, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let envelope: Envelope = serde_json::from_str(line).map_err(|source| WorkerError::InvalidJsonLine {
            line: index + 1,
            source,
        })?;
        validate_envelope_context(&envelope, expected_request_id, expected_command)?;
        match envelope.kind {
            ResponseKind::VersionResult => {
                let payload: WorkerVersionPayload = parse_payload(envelope.payload)?;
                rows.push(Row::Version(payload.into()));
            }
            ResponseKind::DeclarationRow => {
                let payload: DeclarationPayload = parse_payload(envelope.payload)?;
                rows.push(Row::Declaration(payload.into()));
            }
            ResponseKind::FeatureRow => {
                let payload: FeaturePayload = parse_payload(envelope.payload)?;
                rows.push(Row::Feature(payload.into()));
            }
            ResponseKind::ProbeResult => {
                let payload: ProbePayload = parse_payload(envelope.payload)?;
                rows.push(Row::Probe(payload.into()));
            }
            ResponseKind::Progress => {
                let payload: ProgressPayload = parse_payload(envelope.payload)?;
                events.push(payload.into());
            }
            ResponseKind::Error => {
                let payload: ErrorPayload = parse_payload(envelope.payload)?;
                let diagnostic = WorkerDiagnostic {
                    code: payload.code,
                    message: payload.message,
                    fatal: payload.fatal,
                    details: payload.details,
                };
                if diagnostic.fatal {
                    return Err(WorkerError::WorkerDiagnostic {
                        diagnostics: vec![diagnostic],
                    });
                }
                diagnostics.push(diagnostic);
            }
            ResponseKind::Complete => {
                let payload: CompletePayload = parse_payload(envelope.payload)?;
                complete = Some(payload);
            }
            ResponseKind::DoctorResult => {
                return Err(WorkerError::Protocol {
                    message: "doctor_result is not consumed by the Rust worker client".to_owned(),
                });
            }
        }
    }

    let Some(complete_payload) = complete else {
        return Err(WorkerError::EofBeforeComplete { diagnostics });
    };
    validate_row_counts(&complete_payload, &rows, &events, &diagnostics)?;
    Ok(ProtocolOutput {
        rows,
        events,
        diagnostics,
    })
}

fn parse_payload<T: for<'de> Deserialize<'de>>(payload: Value) -> Result<T, WorkerError> {
    serde_json::from_value(payload).map_err(|source| WorkerError::Protocol {
        message: source.to_string(),
    })
}

fn validate_envelope_context(
    envelope: &Envelope,
    expected_request_id: &str,
    expected_command: Command,
) -> Result<(), WorkerError> {
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(WorkerError::Protocol {
            message: format!("unsupported schema `{}`", envelope.schema_version),
        });
    }
    if envelope.extensions.is_some() {
        // Extensions are intentionally accepted but uninterpreted.
    }
    if envelope.kind == ResponseKind::Error {
        return Ok(());
    }
    if envelope.request_id.as_deref() != Some(expected_request_id) {
        return Err(WorkerError::Protocol {
            message: "response request_id did not match request".to_owned(),
        });
    }
    if envelope.command != Some(expected_command) {
        return Err(WorkerError::Protocol {
            message: "response command did not match request".to_owned(),
        });
    }
    Ok(())
}

fn validate_row_counts(
    complete: &CompletePayload,
    rows: &[Row],
    events: &[WorkerEvent],
    diagnostics: &[WorkerDiagnostic],
) -> Result<(), WorkerError> {
    let _elapsed_ms = complete.elapsed_ms;
    let expected = [
        (
            "version_result",
            rows.iter().filter(|row| matches!(row, Row::Version(_))).count(),
        ),
        (
            "declaration_row",
            rows.iter().filter(|row| matches!(row, Row::Declaration(_))).count(),
        ),
        (
            "feature_row",
            rows.iter().filter(|row| matches!(row, Row::Feature(_))).count(),
        ),
        (
            "probe_result",
            rows.iter().filter(|row| matches!(row, Row::Probe(_))).count(),
        ),
        ("progress", events.len()),
        ("error", diagnostics.len()),
    ];
    for (kind, count) in expected {
        let Some(value) = complete.row_counts.get(kind) else {
            if count == 0 {
                continue;
            }
            return Err(WorkerError::Protocol {
                message: format!("complete row_counts omitted `{kind}`"),
            });
        };
        if value.as_u64() != Some(count as u64) {
            return Err(WorkerError::Protocol {
                message: format!("complete row_counts for `{kind}` did not match emitted rows"),
            });
        }
    }
    Ok(())
}

impl From<WorkerVersionPayload> for WorkerVersion {
    fn from(payload: WorkerVersionPayload) -> Self {
        Self {
            protocol_version: payload.protocol_version,
            worker_version: payload.worker_version,
            lean_version: payload.lean_version,
            extract_version: payload.semantic_versions.extract,
            features_version: payload.semantic_versions.features,
            probe_version: payload.semantic_versions.probe,
            supported_commands: payload
                .supported_commands
                .into_iter()
                .map(|command| command.as_str().to_owned())
                .collect(),
            supported_capabilities: payload.supported_capabilities,
        }
    }
}

impl From<DeclarationPayload> for DeclarationRow {
    fn from(payload: DeclarationPayload) -> Self {
        Self {
            declaration_id: payload.declaration_id,
            origin: payload.origin,
            module: payload.module,
            qualified_name: payload.qualified_name,
            display_name: payload.display_name,
            kind: payload.kind,
            visibility: payload.visibility,
            modifiers: payload.modifiers,
            source_span: payload.source_span,
            statement_text: payload.statement_text,
            status_flags: payload.status_flags,
        }
    }
}

impl From<FeaturePayload> for FeatureRow {
    fn from(payload: FeaturePayload) -> Self {
        Self {
            declaration_id: payload.declaration_id,
            feature_version: payload.feature_version,
            fingerprints: payload.fingerprints,
            role_features: payload.role_features,
            binder_count: payload.binder_count,
            low_signal_markers: payload.low_signal_markers,
        }
    }
}

impl From<ProbePayload> for ProbeResult {
    fn from(payload: ProbePayload) -> Self {
        Self {
            pair_id: payload.pair_id,
            left_declaration_id: payload.left_declaration_id,
            right_declaration_id: payload.right_declaration_id,
            status: payload.status,
            same_statement: payload.same_statement,
            same_up_to_safe_reordering: payload.same_up_to_safe_reordering,
            connective_equivalent: payload.connective_equivalent,
            specializes_left_to_right: payload.specializes_left_to_right,
            specializes_right_to_left: payload.specializes_right_to_left,
            mutual_implication_shape: payload.mutual_implication_shape,
            same_reducible_definition: payload.same_reducible_definition,
            message: payload.message,
        }
    }
}

impl From<ProgressPayload> for WorkerEvent {
    fn from(payload: ProgressPayload) -> Self {
        Self {
            phase: payload.phase,
            current: payload.current,
            total: payload.total,
            module: payload.module,
            declaration: payload.declaration,
            elapsed_ms: payload.elapsed_ms,
            message: payload.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_output};
    use crate::worker::WorkerError;

    fn line(kind: &str, payload: &str) -> String {
        format!(
            r#"{{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"{kind}","payload":{payload}}}"#
        )
    }

    #[test]
    fn malformed_json_line_is_structured_error() {
        let error = parse_output("{", "r1", Command::Version).unwrap_err();
        assert!(matches!(error, WorkerError::InvalidJsonLine { line: 1, .. }));
    }

    #[test]
    fn envelope_rejects_unknown_top_level_fields_except_extensions() {
        let bad = r#"{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"complete","payload":{"row_counts":{},"elapsed_ms":null},"surprise":1}"#;
        assert!(matches!(
            parse_output(bad, "r1", Command::Version),
            Err(WorkerError::InvalidJsonLine { .. })
        ));

        let good = r#"{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"complete","payload":{"row_counts":{},"elapsed_ms":null},"extensions":{"x":1}}"#;
        assert!(parse_output(good, "r1", Command::Version).is_ok());
    }

    #[test]
    fn unknown_response_kind_is_rejected() {
        let line = line("not_a_kind", "{}");
        assert!(matches!(
            parse_output(&line, "r1", Command::Version),
            Err(WorkerError::InvalidJsonLine { .. })
        ));
    }

    #[test]
    fn unknown_command_is_rejected() {
        let line = r#"{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"raw","kind":"complete","payload":{"row_counts":{},"elapsed_ms":null}}"#;
        assert!(matches!(
            parse_output(line, "r1", Command::Version),
            Err(WorkerError::InvalidJsonLine { .. })
        ));
    }

    #[test]
    fn missing_required_payload_fields_are_rejected() {
        let stdout = format!(
            "{}\n{}",
            line("version_result", "{}"),
            line("complete", r#"{"row_counts":{"version_result":1},"elapsed_ms":null}"#)
        );
        assert!(matches!(
            parse_output(&stdout, "r1", Command::Version),
            Err(WorkerError::Protocol { .. })
        ));
    }

    #[test]
    fn unknown_supported_command_in_version_payload_is_rejected() {
        let stdout = format!(
            "{}\n{}",
            line(
                "version_result",
                r#"{"protocol_version":"lean-dup.worker.v1","worker_version":"0.1.0","lean_version":null,"semantic_versions":{"extract":"e","features":"f","probe":"p"},"supported_commands":["version","raw"],"supported_capabilities":[]}"#
            ),
            line("complete", r#"{"row_counts":{"version_result":1},"elapsed_ms":null}"#)
        );
        assert!(matches!(
            parse_output(&stdout, "r1", Command::Version),
            Err(WorkerError::Protocol { .. })
        ));
    }

    #[test]
    fn progress_events_are_parsed_and_counted() {
        let stdout = format!(
            "{}\n{}",
            line(
                "progress",
                r#"{"phase":"lean.import","current":2,"total":2,"module":null,"declaration":null,"elapsed_ms":11,"message":"imported requested modules"}"#
            ),
            line("complete", r#"{"row_counts":{"progress":1},"elapsed_ms":null}"#)
        );
        let output = parse_output(&stdout, "r1", Command::Version).unwrap();

        assert_eq!(output.events.len(), 1);
        assert_eq!(output.events[0].phase, "lean.import");
        assert_eq!(output.events[0].elapsed_ms, Some(11));
    }

    #[test]
    fn fatal_worker_error_discards_rows() {
        let stdout = format!(
            "{}\n{}",
            line(
                "version_result",
                r#"{"protocol_version":"lean-dup.worker.v1","worker_version":"0.1.0","lean_version":null,"semantic_versions":{"extract":"e","features":"f","probe":"p"},"supported_commands":["version"],"supported_capabilities":[]}"#
            ),
            line("error", r#"{"code":"internal_error","fatal":true,"message":"failed"}"#)
        );
        assert!(matches!(
            parse_output(&stdout, "r1", Command::Version),
            Err(WorkerError::WorkerDiagnostic { .. })
        ));
    }
}
