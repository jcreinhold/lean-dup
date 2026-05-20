use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use crate::cli::{ALL_BUILT_IN_COMMANDS, Cli, VISIBLE_BUILT_IN_COMMANDS};

const EXTENSION_PREFIX: &str = "lean-dup-";

pub(crate) fn run_external<O, E>(cli: &Cli, external: &[OsString], stdout: &mut O, stderr: &mut E) -> i32
where
    O: Write + Send,
    E: Write + Send,
{
    let Some((name, args)) = external.split_first() else {
        let _ = writeln!(stderr, "error: missing external command name");
        return 1;
    };
    let name = match validate_extension_name(name) {
        Ok(name) => name,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return 1;
        }
    };
    let executable = executable_name(name);
    let Some(path) = find_on_path(&executable) else {
        write_missing_extension(stderr, name, &executable);
        return 1;
    };
    run_extension_process(&path, cli.progress, cli.profile, args, stdout, stderr).unwrap_or_else(|error| {
        let _ = writeln!(stderr, "error: could not run external command `{executable}`: {error}");
        1
    })
}

pub(crate) fn write_command_list<W: Write>(writer: &mut W) -> io::Result<()> {
    writeln!(writer, "lean-dup commands:")?;
    for command in VISIBLE_BUILT_IN_COMMANDS {
        writeln!(writer, "  {command}")?;
    }
    writeln!(writer)?;
    writeln!(writer, "installed extensions:")?;
    let extensions = installed_extensions();
    if extensions.is_empty() {
        writeln!(writer, "  (none)")?;
    } else {
        for extension in extensions {
            writeln!(writer, "  {extension}")?;
        }
    }
    Ok(())
}

fn validate_extension_name(name: &OsStr) -> Result<&str, String> {
    let Some(name) = name.to_str() else {
        return Err("external command name is not valid UTF-8".to_owned());
    };
    if name.is_empty() {
        return Err("external command name is empty".to_owned());
    }
    if name.starts_with('-') || name.starts_with('.') {
        return Err(format!("invalid external command name `{name}`"));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(format!("invalid external command name `{name}`"));
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("external command name is empty".to_owned());
    };
    if !first.is_ascii_alphanumeric() {
        return Err(format!("invalid external command name `{name}`"));
    }
    if chars.any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')) {
        return Err(format!("invalid external command name `{name}`"));
    }
    Ok(name)
}

fn executable_name(name: &str) -> String {
    format!("{EXTENSION_PREFIX}{name}")
}

fn write_missing_extension<W: Write>(writer: &mut W, name: &str, executable: &str) {
    let _ = writeln!(writer, "error: external command `{executable}` was not found on PATH");
    if name == "vector" {
        let _ = writeln!(writer, "help: install it with `cargo install lean-dup-vector-search`");
    } else {
        let _ = writeln!(writer, "help: install an executable named `{executable}` on PATH");
    }
}

fn run_extension_process<O, E>(
    path: &Path,
    progress: bool,
    profile: bool,
    args: &[OsString],
    stdout: &mut O,
    stderr: &mut E,
) -> io::Result<i32>
where
    O: Write + Send,
    E: Write + Send,
{
    let mut command = ProcessCommand::new(path);
    if progress {
        command.arg("--progress");
    }
    if profile {
        command.arg("--profile");
    }
    command
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("external command stdout was not captured"))?;
    let mut child_stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("external command stderr was not captured"))?;
    let status = std::thread::scope(|scope| {
        let stdout_thread = scope.spawn(|| io::copy(&mut child_stdout, stdout));
        let stderr_thread = scope.spawn(|| io::copy(&mut child_stderr, stderr));
        let status = child.wait()?;
        join_copy(stdout_thread)?;
        join_copy(stderr_thread)?;
        Ok::<std::process::ExitStatus, io::Error>(status)
    })?;
    Ok(status.code().unwrap_or(1))
}

fn join_copy(thread: std::thread::ScopedJoinHandle<'_, io::Result<u64>>) -> io::Result<()> {
    thread
        .join()
        .map_err(|_| io::Error::other("external command output thread panicked"))?
        .map(|_| ())
}

fn installed_extensions() -> Vec<String> {
    let built_ins = ALL_BUILT_IN_COMMANDS.iter().copied().collect::<BTreeSet<_>>();
    let mut extensions = BTreeSet::new();
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    for directory in std::env::split_paths(&path) {
        collect_extensions_from_dir(&directory, &built_ins, &mut extensions);
    }
    extensions.into_iter().collect()
}

fn collect_extensions_from_dir(directory: &Path, built_ins: &BTreeSet<&str>, extensions: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        let Some(name) = file_name.strip_prefix(EXTENSION_PREFIX) else {
            continue;
        };
        if built_ins.contains(name) || validate_extension_name(OsStr::new(name)).is_err() {
            continue;
        }
        extensions.insert(name.to_owned());
    }
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(executable);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
