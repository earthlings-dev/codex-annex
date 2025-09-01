// annex/src/git_hooks.rs

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn install_pre_commit(repo_root: &Path) -> anyhow::Result<()> {
    let hooks = repo_root.join(".git/hooks");
    fs::create_dir_all(&hooks)?;
    let script = hooks.join("pre-commit");
    let body = r#"#!/bin/sh
# Minimal pre-commit hook: emit codex event; ignore failures.
codex --emit-hook git:pre-commit || true
"#;
    fs::write(&script, body)?;
    let mut perm = fs::metadata(&script)?.permissions();
    perm.set_mode(0o755);
    fs::set_permissions(script, perm)?;
    Ok(())
}