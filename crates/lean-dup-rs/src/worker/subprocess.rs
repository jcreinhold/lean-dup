use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::WorkerError;
use super::protocol::{self, ProtocolItem, ProtocolOutput, Request, StreamParser};
use super::transport::{CallControl, WorkerTransport};
use crate::perf::{self, CostClass};

const STDERR_LIMIT: usize = 8 * 1024;

#[derive(Debug)]
pub(super) struct SubprocessTransport {
    worker_root: PathBuf,
    command_override: Option<TestCommand>,
}

#[derive(Debug, Clone)]
struct TestCommand {
    cwd: PathBuf,
    program: String,
    args: Vec<String>,
}

#[derive(Debug)]
struct ProcessOutput {
    status: i32,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug)]
struct StreamProcessOutput {
    status: i32,
    stderr: String,
    timed_out: bool,
    events: Vec<super::WorkerEvent>,
    diagnostics: Vec<super::WorkerDiagnostic>,
}

impl SubprocessTransport {
    pub(super) fn new() -> Self {
        Self {
            worker_root: repo_root().join("lean"),
            command_override: None,
        }
    }

    #[cfg(test)]
    pub(super) fn for_test(cwd: PathBuf, program: String, args: Vec<String>) -> Self {
        Self {
            worker_root: cwd.clone(),
            command_override: Some(TestCommand { cwd, program, args }),
        }
    }

    fn build_worker(&self, control: &CallControl) -> Result<PathBuf, WorkerError> {
        if self.command_override.is_some() {
            return Ok(PathBuf::new());
        }
        let worker_path = self.worker_root.join(".lake/build/bin/lean_dup_worker");
        let mut cache =
            WORKER_PATH_CACHE
                .get_or_init(|| Mutex::new(None))
                .lock()
                .map_err(|_| WorkerError::Protocol {
                    message: "worker build cache mutex poisoned".to_owned(),
                })?;
        if !worker_build_cache_disabled() && cache.as_ref() == Some(&worker_path) && worker_path.exists() {
            perf::record_count(CostClass::WorkerStartup, "worker.build.cache_hit", 1);
            return Ok(worker_path);
        }
        let output = perf::measure_result(CostClass::WorkerStartup, "worker.build_process", || {
            run_process(
                &self.worker_root,
                "lake",
                &["build".to_owned(), "lean_dup_worker".to_owned()],
                "",
                control,
            )
        })?;
        if output.timed_out {
            return Err(WorkerError::Timeout {
                timeout: control.timeout,
            });
        }
        if output.status != 0 {
            return Err(WorkerError::BuildFailed {
                status: output.status,
                diagnostic: output.stderr,
            });
        }
        if !worker_build_cache_disabled() {
            *cache = Some(worker_path.clone());
        }
        Ok(worker_path)
    }

    fn invoke_worker(&self, request: &Request, control: &CallControl) -> Result<ProcessOutput, WorkerError> {
        let input = perf::measure_result(CostClass::Transport, "worker.encode_json", || {
            serde_json::to_string(&request.to_json()).map_err(|source| WorkerError::Protocol {
                message: source.to_string(),
            })
        })?;
        perf::record_count(CostClass::Transport, "worker.stdin_bytes", input.len() as u64);
        if let Some(command) = &self.command_override {
            return run_process(&command.cwd, &command.program, &command.args, &input, control);
        }
        let worker_path = self.build_worker(control)?;
        let worker = worker_path.to_string_lossy().into_owned();
        perf::measure_result(CostClass::WorkerStartup, "worker.subprocess_call", || {
            run_process(
                &request_workspace(request)?,
                "lake",
                &["env".to_owned(), worker],
                &input,
                control,
            )
        })
    }
}

impl WorkerTransport for SubprocessTransport {
    fn call(&self, request: Request, control: CallControl) -> Result<ProtocolOutput, WorkerError> {
        let output = self.invoke_worker(&request, &control)?;
        if output.timed_out {
            return Err(WorkerError::Timeout {
                timeout: control.timeout,
            });
        }
        perf::record_count(CostClass::Transport, "worker.stdout_bytes", output.stdout.len() as u64);
        perf::record_count(
            CostClass::Transport,
            "worker.stdout_lines",
            output.stdout.lines().count() as u64,
        );
        let parsed = perf::measure_result(CostClass::Transport, "worker.parse_jsonl", || {
            protocol::parse_output(&output.stdout, &request.request_id, request.command)
        });
        if output.status != 0 {
            if let Err(error @ WorkerError::WorkerDiagnostic { .. }) = parsed {
                return Err(error);
            }
            return Err(WorkerError::NonZeroExit {
                status: output.status,
                stderr: output.stderr,
            });
        }
        parsed
    }

    fn call_stream(
        &self,
        request: Request,
        control: CallControl,
        sink: &mut dyn FnMut(ProtocolItem) -> Result<(), WorkerError>,
    ) -> Result<ProtocolOutput, WorkerError> {
        let input = perf::measure_result(CostClass::Transport, "worker.encode_json", || {
            serde_json::to_string(&request.to_json()).map_err(|source| WorkerError::Protocol {
                message: source.to_string(),
            })
        })?;
        perf::record_count(CostClass::Transport, "worker.stdin_bytes", input.len() as u64);
        let output = if let Some(command) = &self.command_override {
            run_process_streaming(
                &command.cwd,
                &command.program,
                &command.args,
                &input,
                &control,
                &request,
                sink,
            )?
        } else {
            let worker_path = self.build_worker(&control)?;
            let worker = worker_path.to_string_lossy().into_owned();
            perf::measure_result(CostClass::WorkerStartup, "worker.subprocess_call", || {
                run_process_streaming(
                    &request_workspace(&request)?,
                    "lake",
                    &["env".to_owned(), worker],
                    &input,
                    &control,
                    &request,
                    sink,
                )
            })?
        };
        if output.timed_out {
            return Err(WorkerError::Timeout {
                timeout: control.timeout,
            });
        }
        if output.status != 0 {
            return Err(WorkerError::NonZeroExit {
                status: output.status,
                stderr: output.stderr,
            });
        }
        Ok(ProtocolOutput {
            rows: Vec::new(),
            events: output.events,
            diagnostics: output.diagnostics,
        })
    }
}

static WORKER_PATH_CACHE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn worker_build_cache_disabled() -> bool {
    std::env::var_os("LEAN_DUP_DISABLE_WORKER_BUILD_CACHE").is_some()
}

fn request_workspace(request: &Request) -> Result<PathBuf, WorkerError> {
    match request.to_json().get("workspace_root").and_then(|value| value.as_str()) {
        Some(root) if !root.is_empty() => Ok(PathBuf::from(root)),
        _ => Ok(repo_root().join("lean")),
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under repo/crates/lean-dup-rs")
        .to_path_buf()
}

fn run_process(
    cwd: &Path,
    program: &str,
    args: &[String],
    stdin: &str,
    control: &CallControl,
) -> Result<ProcessOutput, WorkerError> {
    if control.cancelled.load(Ordering::Relaxed) {
        return Err(WorkerError::Cancelled);
    }
    let mut child = ProcessCommand::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| WorkerError::Io {
            message: format!("could not start `{program}`"),
            source,
        })?;

    let mut child_stdin = child.stdin.take();
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdout_handle = thread::spawn(move || read_stream(stdout, usize::MAX));
    let stderr_handle = thread::spawn(move || read_stream(stderr, STDERR_LIMIT));

    if let Some(mut child_stdin) = child_stdin.take() {
        child_stdin
            .write_all(stdin.as_bytes())
            .map_err(|source| WorkerError::Io {
                message: "could not write worker request".to_owned(),
                source,
            })?;
    }

    let started = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().map_err(|source| WorkerError::Io {
            message: "could not wait for worker process".to_owned(),
            source,
        })? {
            break (status.code().unwrap_or(1), false);
        }
        if control.cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(WorkerError::Cancelled);
        }
        if started.elapsed() > control.timeout {
            let _ = child.kill();
            let _ = child.wait();
            break (1, true);
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = stdout_handle
        .join()
        .map_err(|_| WorkerError::Protocol {
            message: "stdout reader thread panicked".to_owned(),
        })?
        .map_err(|source| WorkerError::Io {
            message: "could not read worker stdout".to_owned(),
            source,
        })?;
    let stderr = stderr_handle
        .join()
        .map_err(|_| WorkerError::Protocol {
            message: "stderr reader thread panicked".to_owned(),
        })?
        .map_err(|source| WorkerError::Io {
            message: "could not read worker stderr".to_owned(),
            source,
        })?;

    Ok(ProcessOutput {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

fn run_process_streaming(
    cwd: &Path,
    program: &str,
    args: &[String],
    stdin: &str,
    control: &CallControl,
    request: &Request,
    sink: &mut dyn FnMut(ProtocolItem) -> Result<(), WorkerError>,
) -> Result<StreamProcessOutput, WorkerError> {
    if control.cancelled.load(Ordering::Relaxed) {
        return Err(WorkerError::Cancelled);
    }
    let mut child = ProcessCommand::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| WorkerError::Io {
            message: format!("could not start `{program}`"),
            source,
        })?;

    if let Some(mut child_stdin) = child.stdin.take() {
        child_stdin
            .write_all(stdin.as_bytes())
            .map_err(|source| WorkerError::Io {
                message: "could not write worker request".to_owned(),
                source,
            })?;
    }

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let (line_tx, line_rx) = mpsc::channel();
    let stdout_handle = thread::spawn(move || read_lines(stdout, line_tx));
    let stderr_handle = thread::spawn(move || read_stream(stderr, STDERR_LIMIT));
    let mut parser = StreamParser::new(request.request_id.clone(), request.command);
    let mut events = Vec::new();
    let mut diagnostics = Vec::new();
    let mut stdout_bytes = 0_u64;
    let mut stdout_lines = 0_u64;
    let started = Instant::now();
    let mut parser_error = None;

    let (status, timed_out) = loop {
        match line_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Ok(line)) => {
                stdout_bytes += line.len() as u64;
                stdout_lines += 1;
                if let Err(error) = process_stream_line(
                    &mut parser,
                    line,
                    stdout_lines as usize,
                    &mut events,
                    &mut diagnostics,
                    sink,
                ) {
                    parser_error = Some(error);
                    let _ = child.kill();
                }
            }
            Ok(Err(source)) => {
                parser_error = Some(WorkerError::Io {
                    message: "could not read worker stdout".to_owned(),
                    source,
                });
                let _ = child.kill();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }

        if let Some(status) = child.try_wait().map_err(|source| WorkerError::Io {
            message: "could not wait for worker process".to_owned(),
            source,
        })? {
            break (status.code().unwrap_or(1), false);
        }
        if parser_error.is_some() {
            let _ = child.wait();
            break (1, false);
        }
        if control.cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(WorkerError::Cancelled);
        }
        if started.elapsed() > control.timeout {
            let _ = child.kill();
            let _ = child.wait();
            break (1, true);
        }
    };

    for line in line_rx.try_iter() {
        let line = line.map_err(|source| WorkerError::Io {
            message: "could not read worker stdout".to_owned(),
            source,
        })?;
        stdout_bytes += line.len() as u64;
        stdout_lines += 1;
        if let Err(error) = process_stream_line(
            &mut parser,
            line,
            stdout_lines as usize,
            &mut events,
            &mut diagnostics,
            sink,
        ) {
            parser_error = Some(error);
        }
    }

    let stdout_result = stdout_handle.join().map_err(|_| WorkerError::Protocol {
        message: "stdout reader thread panicked".to_owned(),
    })?;
    if parser_error.is_none()
        && let Err(source) = stdout_result
    {
        parser_error = Some(WorkerError::Io {
            message: "could not read worker stdout".to_owned(),
            source,
        });
    }
    let stderr = stderr_handle
        .join()
        .map_err(|_| WorkerError::Protocol {
            message: "stderr reader thread panicked".to_owned(),
        })?
        .map_err(|source| WorkerError::Io {
            message: "could not read worker stderr".to_owned(),
            source,
        })?;

    perf::record_count(CostClass::Transport, "worker.stdout_bytes", stdout_bytes);
    perf::record_count(CostClass::Transport, "worker.stdout_lines", stdout_lines);
    if let Some(error) = parser_error {
        return Err(error);
    }
    if status == 0 {
        parser.finish()?;
    }

    Ok(StreamProcessOutput {
        status,
        stderr,
        timed_out,
        events,
        diagnostics,
    })
}

fn process_stream_line(
    parser: &mut StreamParser,
    line: String,
    line_number: usize,
    events: &mut Vec<super::WorkerEvent>,
    diagnostics: &mut Vec<super::WorkerDiagnostic>,
    sink: &mut dyn FnMut(ProtocolItem) -> Result<(), WorkerError>,
) -> Result<(), WorkerError> {
    match parser.accept_line(line.trim_end_matches(['\r', '\n']), line_number)? {
        Some(ProtocolItem::Event(event)) => {
            events.push(event.clone());
            sink(ProtocolItem::Event(event))
        }
        Some(ProtocolItem::Diagnostic(diagnostic)) => {
            diagnostics.push(diagnostic.clone());
            sink(ProtocolItem::Diagnostic(diagnostic))
        }
        Some(item) => sink(item),
        None => Ok(()),
    }
}

fn read_lines(stream: impl Read, sender: mpsc::Sender<std::io::Result<String>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        if sender.send(Ok(line)).is_err() {
            break;
        }
    }
    Ok(())
}

fn read_stream(mut stream: impl Read, limit: usize) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        bytes.truncate(limit);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::SubprocessTransport;
    use crate::worker::WorkerError;
    use crate::worker::protocol::{Command, ProtocolItem, Request};
    use crate::worker::transport::{CallControl, WorkerTransport};

    fn script(body: &str) -> (TempDir, SubprocessTransport) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("worker.sh");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let transport = SubprocessTransport::for_test(
            temp.path().to_path_buf(),
            "sh".to_owned(),
            vec![path.to_string_lossy().into_owned()],
        );
        (temp, transport)
    }

    fn request(command: Command) -> Request {
        Request::new("r1".to_owned(), command, serde_json::json!({}))
    }

    fn control() -> CallControl {
        CallControl {
            timeout: std::time::Duration::from_secs(2),
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn malformed_json_is_structured_failure() {
        let (_temp, transport) = script("printf '{'");
        assert!(matches!(
            transport.call(request(Command::Version), control()),
            Err(WorkerError::InvalidJsonLine { .. })
        ));
    }

    #[test]
    fn unknown_response_kind_is_structured_failure() {
        let (_temp, transport) = script(
            r#"printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"mystery","payload":{}}'"#,
        );
        assert!(matches!(
            transport.call(request(Command::Version), control()),
            Err(WorkerError::InvalidJsonLine { .. })
        ));
    }

    #[test]
    fn eof_before_complete_discards_rows() {
        let (_temp, transport) = script(
            r#"printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"version_result","payload":{"protocol_version":"lean-dup.worker.v1","worker_version":"0.1.0","lean_version":null,"semantic_versions":{"extract":"e","features":"f","probe":"p"},"supported_commands":["version"],"supported_capabilities":[]}}'"#,
        );
        assert!(matches!(
            transport.call(request(Command::Version), control()),
            Err(WorkerError::EofBeforeComplete { .. })
        ));
    }

    #[test]
    fn nonzero_exit_discards_partial_rows() {
        let (_temp, transport) = script(
            r#"printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"version_result","payload":{"protocol_version":"lean-dup.worker.v1","worker_version":"0.1.0","lean_version":null,"semantic_versions":{"extract":"e","features":"f","probe":"p"},"supported_commands":["version"],"supported_capabilities":[]}}'
exit 7"#,
        );
        assert!(matches!(
            transport.call(request(Command::Version), control()),
            Err(WorkerError::NonZeroExit { status: 7, .. })
        ));
    }

    #[test]
    fn nonzero_exit_preserves_fatal_worker_diagnostic() {
        let (_temp, transport) = script(
            r#"printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"error","payload":{"code":"internal_error","fatal":true,"message":"maximum number of heartbeats has been reached","details":null}}'
exit 1"#,
        );
        let error = transport.call(request(Command::Version), control()).unwrap_err();
        match error {
            WorkerError::WorkerDiagnostic { diagnostics } => {
                assert_eq!(diagnostics[0].code, "internal_error");
                assert!(diagnostics[0].message.contains("heartbeats"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn stderr_is_bounded() {
        let (_temp, transport) = script("python3 - <<'PY'\nimport sys\nsys.stderr.write('x' * 12000)\nsys.exit(3)\nPY");
        let error = transport.call(request(Command::Version), control()).unwrap_err();
        match error {
            WorkerError::NonZeroExit { stderr, .. } => assert!(stderr.len() <= 8192),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn timeout_terminates_worker() {
        let (_temp, transport) = script("sleep 5");
        let error = transport
            .call(
                request(Command::Version),
                CallControl {
                    timeout: std::time::Duration::from_millis(20),
                    cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                },
            )
            .unwrap_err();
        assert!(matches!(error, WorkerError::Timeout { .. }));
    }

    #[test]
    fn cancellation_terminates_before_start() {
        let (_temp, transport) = script("sleep 5");
        let error = transport
            .call(
                request(Command::Version),
                CallControl {
                    timeout: std::time::Duration::from_secs(2),
                    cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
                },
            )
            .unwrap_err();
        assert!(matches!(error, WorkerError::Cancelled));
    }

    #[test]
    fn rows_commit_after_complete() {
        let (_temp, transport) = script(
            r#"printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"version_result","payload":{"protocol_version":"lean-dup.worker.v1","worker_version":"0.1.0","lean_version":null,"semantic_versions":{"extract":"e","features":"f","probe":"p"},"supported_commands":["version"],"supported_capabilities":[]}}'
printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"version","kind":"complete","payload":{"row_counts":{"version_result":1},"elapsed_ms":null}}'"#,
        );
        let output = transport.call(request(Command::Version), control()).unwrap();
        assert_eq!(output.rows.len(), 1);
    }

    #[test]
    fn streamed_progress_is_delivered_before_complete() {
        let (_temp, transport) = script(
            r#"printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"index","kind":"progress","payload":{"phase":"lean.index.chunk","current":1,"total":2,"module":null,"declaration":null,"elapsed_ms":7,"message":"first chunk"}}'
printf '%s\n' '{"schema_version":"lean-dup.worker.v1","request_id":"r1","command":"index","kind":"complete","payload":{"row_counts":{"progress":1},"elapsed_ms":null}}'"#,
        );
        let mut seen = Vec::new();
        let output = transport
            .call_stream(request(Command::Index), control(), &mut |item| {
                match item {
                    ProtocolItem::Event(event) => seen.push(event.phase),
                    ProtocolItem::Complete => seen.push("complete".to_owned()),
                    _ => {}
                }
                Ok(())
            })
            .unwrap();

        assert_eq!(seen, vec!["lean.index.chunk", "complete"]);
        assert_eq!(output.events.len(), 1);
    }
}
