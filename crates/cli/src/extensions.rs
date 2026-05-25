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
    let width = VISIBLE_BUILT_IN_COMMANDS.iter().map(|cmd| cmd.len()).max().unwrap_or(0);
    for command in VISIBLE_BUILT_IN_COMMANDS {
        let about = built_in_about(command).unwrap_or("");
        writeln!(writer, "  {command:<width$}  {about}")?;
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

/// One-line description for each visible built-in subcommand. Mirrors the
/// doc-comment `about` strings on `Command` variants in `cli.rs`; kept in
/// sync by hand since clap-derive doesn't expose them at runtime.
fn built_in_about(name: &str) -> Option<&'static str> {
    match name {
        "doctor" => Some("Diagnose workspace, cache, and worker health."),
        "cache-cleanup" => Some("Remove cache entries no workspace points to anymore."),
        "index" => Some("Build or refresh a labelled index for a workspace."),
        "index-mathlib" => Some("Build or refresh the project's mathlib index."),
        "audit" => Some("Find duplicate declarations across the selected workspace."),
        "eval" => Some("Run the recall/precision evaluation suites."),
        "show" => Some("Print the full evidence for one duplicate group."),
        "diff" => Some("Compare current findings against a saved baseline."),
        _ => None,
    }
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
    if !is_known_external(name)
        && let Some(suggestion) = nearest_built_in(name)
    {
        let _ = writeln!(writer, "error: unknown command `{name}`");
        let _ = writeln!(writer, "help: did you mean `{suggestion}`?");
        return;
    }
    let _ = writeln!(writer, "error: external command `{executable}` was not found on PATH");
    if name == "vector" {
        let _ = writeln!(writer, "help: install it with `cargo install lean-dup-vector-search`");
    } else {
        let _ = writeln!(writer, "help: install an executable named `{executable}` on PATH");
    }
}

/// Names that should be treated as external extensions even when they happen
/// to fall within edit distance of a built-in (e.g. `vector` vs `doctor`).
fn is_known_external(name: &str) -> bool {
    matches!(name, "vector")
}

/// Return the closest built-in subcommand to `name` if it is within edit
/// distance 2, otherwise None. Used to suggest `audit` for `audot` before
/// falling through to the external-extension lookup.
fn nearest_built_in(name: &str) -> Option<&'static str> {
    const MAX_DISTANCE: usize = 2;
    let mut best: Option<(&'static str, usize)> = None;
    for &candidate in ALL_BUILT_IN_COMMANDS {
        let distance = levenshtein(name, candidate);
        if distance > MAX_DISTANCE {
            continue;
        }
        match best {
            Some((_, current)) if distance >= current => {}
            _ => best = Some((candidate, distance)),
        }
    }
    best.map(|(candidate, _)| candidate)
}

/// Return the candidate closest to `name` within `max_distance` edits, if any.
/// Used by both subcommand dispatch and group-ID fast-fail.
pub(crate) fn nearest_match<'a, I>(name: &str, candidates: I, max_distance: usize) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(&'a str, usize)> = None;
    for candidate in candidates {
        let distance = levenshtein(name, candidate);
        if distance > max_distance {
            continue;
        }
        match best {
            Some((_, current)) if distance >= current => {}
            _ => best = Some((candidate, distance)),
        }
    }
    best.map(|(candidate, _)| candidate)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ch_a) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, ch_b) in b.iter().enumerate() {
            let cost = if ch_a == ch_b { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_built_in_suggests_audit_for_audot() {
        assert_eq!(nearest_built_in("audot"), Some("audit"));
    }

    #[test]
    fn nearest_built_in_suggests_doctor_for_doctr() {
        assert_eq!(nearest_built_in("doctr"), Some("doctor"));
    }

    #[test]
    fn nearest_built_in_returns_none_for_distant_strings() {
        assert_eq!(nearest_built_in("xyzzy"), None);
        assert_eq!(nearest_built_in("zorp"), None);
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
