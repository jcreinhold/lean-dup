use std::path::{Path, PathBuf};
use std::process::Command;

use lean_dup_search::AuditSourceRange;

use crate::error::{AppError, Result};

/// Resolve the current-line hunks changed from `revision` into absolute Lean
/// source ranges. Git remains a CLI concern; search receives only source facts.
pub(crate) fn source_ranges(workspace: &Path, revision: &str) -> Result<Vec<AuditSourceRange>> {
    if revision.is_empty() || revision.starts_with('-') {
        return Err(AppError::Cli {
            message: format!("invalid --changed-since revision `{revision}`"),
        });
    }
    let root_output = git(workspace, ["rev-parse", "--show-toplevel"])?;
    let root_text = String::from_utf8(root_output).map_err(|_| AppError::Cli {
        message: "git returned a non-UTF-8 repository root".to_owned(),
    })?;
    let root = PathBuf::from(root_text.trim());
    let names = git(
        &root,
        [
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=ACMRTUXB",
            revision,
            "--",
            "*.lean",
        ],
    )?;
    let untracked = git(
        &root,
        ["ls-files", "--others", "--exclude-standard", "-z", "--", "*.lean"],
    )?;
    let mut ranges = Vec::new();
    for name in names.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
        let relative = String::from_utf8(name.to_vec()).map_err(|_| AppError::Cli {
            message: "git returned a non-UTF-8 Lean source path".to_owned(),
        })?;
        let current_file = root.join(&relative);
        if !current_file.is_file() {
            continue;
        }
        let patch = git(
            &root,
            [
                "diff",
                "--unified=0",
                "--no-color",
                "--no-ext-diff",
                revision,
                "--",
                &relative,
            ],
        )?;
        let patch = String::from_utf8(patch).map_err(|_| AppError::Cli {
            message: format!("git returned a non-UTF-8 patch for `{relative}`"),
        })?;
        let file = current_file.canonicalize().map_err(|source| AppError::Io {
            message: "could not resolve changed Lean source",
            path: current_file.clone(),
            source,
        })?;
        ranges.extend(
            parse_current_ranges(&patch)
                .into_iter()
                .map(|(start_line, end_line)| AuditSourceRange {
                    file: file.clone(),
                    start_line,
                    end_line,
                }),
        );
    }
    for name in untracked.split(|byte| *byte == 0).filter(|name| !name.is_empty()) {
        let relative = String::from_utf8(name.to_vec()).map_err(|_| AppError::Cli {
            message: "git returned a non-UTF-8 Lean source path".to_owned(),
        })?;
        let current_file = root.join(relative);
        if !current_file.is_file() {
            continue;
        }
        let file = current_file.canonicalize().map_err(|source| AppError::Io {
            message: "could not resolve untracked Lean source",
            path: current_file,
            source,
        })?;
        ranges.push(AuditSourceRange {
            file,
            start_line: 1,
            end_line: u64::MAX,
        });
    }
    ranges.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then_with(|| left.start_line.cmp(&right.start_line))
            .then_with(|| left.end_line.cmp(&right.end_line))
    });
    ranges.dedup();
    Ok(ranges)
}

fn git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|source| AppError::Io {
            message: "could not run git for --changed-since",
            path: cwd.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(AppError::Cli {
        message: if stderr.is_empty() {
            "git failed while resolving --changed-since".to_owned()
        } else {
            format!("git failed while resolving --changed-since: {stderr}")
        },
    })
}

fn parse_current_ranges(patch: &str) -> Vec<(u64, u64)> {
    patch
        .lines()
        .filter_map(|line| line.strip_prefix("@@ "))
        .filter_map(|header| header.split_whitespace().find(|part| part.starts_with('+')))
        .filter_map(parse_current_range)
        .collect()
}

fn parse_current_range(spec: &str) -> Option<(u64, u64)> {
    let spec = spec.strip_prefix('+')?;
    let (start, count) = match spec.split_once(',') {
        Some((start, count)) => (start.parse::<u64>().ok()?, count.parse::<u64>().ok()?),
        None => (spec.parse::<u64>().ok()?, 1),
    };
    if count == 0 {
        return None;
    }
    let end = start.checked_add(count)?.checked_sub(1)?;
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::{parse_current_range, parse_current_ranges, source_ranges};

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git").current_dir(cwd).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn current_hunks_map_to_inclusive_ranges() {
        let patch = "@@ -4,2 +4,3 @@\n-old\n+new\n@@ -20 +21 @@\n-old\n+new\n";
        assert_eq!(parse_current_ranges(patch), vec![(4, 6), (21, 21)]);
    }

    #[test]
    fn deletion_only_hunks_do_not_focus_current_source() {
        assert_eq!(parse_current_ranges("@@ -4,3 +4,0 @@\n-old\n"), vec![]);
    }

    #[test]
    fn malformed_or_overflowing_hunks_are_ignored() {
        assert_eq!(parse_current_range("+not-a-line,2"), None);
        assert_eq!(parse_current_range("+18446744073709551615,2"), None);
    }

    #[test]
    fn git_changes_focus_only_current_lean_hunks() {
        let temp = tempfile::TempDir::new().unwrap();
        git(temp.path(), &["init", "-q"]);
        git(temp.path(), &["config", "user.email", "lean-dup@example.invalid"]);
        git(temp.path(), &["config", "user.name", "lean-dup tests"]);
        let lean = temp.path().join("Example.lean");
        fs::write(
            &lean,
            "theorem first : True := trivial\ntheorem second : True := trivial\n",
        )
        .unwrap();
        git(temp.path(), &["add", "Example.lean"]);
        git(temp.path(), &["commit", "-q", "-m", "fixture"]);
        fs::write(
            &lean,
            "theorem first : True := trivial\ntheorem second : True := by trivial\n",
        )
        .unwrap();

        let ranges = source_ranges(temp.path(), "HEAD").unwrap();

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].file, lean.canonicalize().unwrap());
        assert_eq!((ranges[0].start_line, ranges[0].end_line), (2, 2));
    }

    #[test]
    fn git_changes_include_untracked_lean_files_but_not_other_files() {
        let temp = tempfile::TempDir::new().unwrap();
        git(temp.path(), &["init", "-q"]);
        git(temp.path(), &["config", "user.email", "lean-dup@example.invalid"]);
        git(temp.path(), &["config", "user.name", "lean-dup tests"]);
        fs::write(temp.path().join("tracked.txt"), "tracked\n").unwrap();
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-q", "-m", "fixture"]);
        let lean = temp.path().join("New.lean");
        fs::write(&lean, "theorem newTheorem : True := trivial\n").unwrap();
        fs::write(temp.path().join("ignored.txt"), "not Lean\n").unwrap();

        let ranges = source_ranges(temp.path(), "HEAD").unwrap();

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].file, lean.canonicalize().unwrap());
        assert_eq!((ranges[0].start_line, ranges[0].end_line), (1, u64::MAX));
    }
}
