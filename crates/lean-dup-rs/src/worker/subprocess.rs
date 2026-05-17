use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use super::WorkerError;
use super::protocol::{self, ProtocolOutput, Request};
use super::transport::{CallControl, WorkerTransport};

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
        let output = run_process(
            &self.worker_root,
            "lake",
            &["build".to_owned(), "lean_dup_worker".to_owned()],
            "",
            control,
        )?;
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
        Ok(self.worker_root.join(".lake/build/bin/lean_dup_worker"))
    }

    fn invoke_worker(
        &self,
        request: &Request,
        control: &CallControl,
    ) -> Result<ProcessOutput, WorkerError> {
        let input =
            serde_json::to_string(&request.to_json()).map_err(|source| WorkerError::Protocol {
                message: source.to_string(),
            })?;
        if let Some(command) = &self.command_override {
            return run_process(
                &command.cwd,
                &command.program,
                &command.args,
                &input,
                control,
            );
        }
        let worker_path = self.build_worker(control)?;
        let worker = worker_path.to_string_lossy().into_owned();
        run_process(
            &request_workspace(request)?,
            "lake",
            &["env".to_owned(), worker],
            &input,
            control,
        )
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
        let parsed = protocol::parse_output(&output.stdout, &request.request_id, request.command);
        if output.status != 0 {
            return Err(WorkerError::NonZeroExit {
                status: output.status,
                stderr: output.stderr,
            });
        }
        parsed
    }
}

fn request_workspace(request: &Request) -> Result<PathBuf, WorkerError> {
    match request
        .to_json()
        .get("workspace_root")
        .and_then(|value| value.as_str())
    {
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
    use crate::worker::protocol::{Command, Request};
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
    fn stderr_is_bounded() {
        let (_temp, transport) =
            script("python3 - <<'PY'\nimport sys\nsys.stderr.write('x' * 12000)\nsys.exit(3)\nPY");
        let error = transport
            .call(request(Command::Version), control())
            .unwrap_err();
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
        let output = transport
            .call(request(Command::Version), control())
            .unwrap();
        assert_eq!(output.rows.len(), 1);
    }
}
