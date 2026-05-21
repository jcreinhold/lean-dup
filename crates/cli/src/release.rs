use std::io::Write;

use lean_dup_index::{CACHE_KEY_VERSION, INDEX_DIAGNOSTIC_SCHEMA_VERSION};
use lean_dup_report::REPORT_SCHEMA_VERSION;

/// Release identity facts printed by `lean-dup --version` and embedded in
/// `doctor`.
///
/// These facts identify the binary and stable report/cache/index contracts
/// without exposing build-script internals, local repository paths, or cache
/// layout.
pub(crate) fn identity() -> lean_dup_report::ReleaseIdentityReport {
    lean_dup_report::ReleaseIdentityReport {
        package: env!("CARGO_PKG_NAME").to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_revision: option_env!("LEAN_DUP_GIT_REVISION")
            .filter(|revision| !revision.is_empty())
            .unwrap_or("unknown")
            .to_owned(),
        build_profile: if cfg!(debug_assertions) {
            "debug".to_owned()
        } else {
            "release".to_owned()
        },
        report_schema_version: REPORT_SCHEMA_VERSION.to_owned(),
        index_schema_version: INDEX_DIAGNOSTIC_SCHEMA_VERSION.to_owned(),
        cache_key_version: CACHE_KEY_VERSION.to_owned(),
    }
}

pub(crate) fn write_version<W: Write>(stdout: &mut W) -> std::io::Result<()> {
    let identity = identity();
    writeln!(stdout, "lean-dup {}", identity.version)?;
    writeln!(stdout, "package: {}", identity.package)?;
    writeln!(stdout, "git revision: {}", identity.git_revision)?;
    writeln!(stdout, "build profile: {}", identity.build_profile)?;
    writeln!(stdout, "report schema: {}", identity.report_schema_version)?;
    writeln!(stdout, "index schema: {}", identity.index_schema_version)?;
    writeln!(stdout, "cache key: {}", identity.cache_key_version)?;
    writeln!(
        stdout,
        "worker: run `lean-dup doctor --workspace <workspace> --format json` for Lean worker facts"
    )?;
    Ok(())
}
