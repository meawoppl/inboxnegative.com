//! Captures build provenance so the running binary can report which revision it
//! actually loaded, rather than leaving that to be inferred from an image label.
//! An image label describes the image; it is one indirection away from what the
//! process is really running.
//!
//! Resolution order for the revision:
//!   1. `BUILD_REVISION` env var, so a builder that knows the SHA can state it
//!      outright (CI has `github.sha` and need not depend on git being present).
//!   2. `git rev-parse HEAD`, with `-dirty` appended for uncommitted changes.
//!   3. `"unknown"`.
//!
//! Never fails the build. A missing git binary or absent `.git` yields "unknown";
//! provenance is diagnostic, and refusing to compile over it would be worse than
//! not having it.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=BUILD_REVISION");

    // Re-run when HEAD moves, so a rebuild with no source change still picks up a
    // new commit. The workspace `.git` is one level up from this crate.
    for path in ["../.git/HEAD", ".git/HEAD"] {
        if std::path::Path::new(path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }

    println!("cargo:rustc-env=BUILD_REVISION={}", revision());
}

fn revision() -> String {
    if let Ok(rev) = std::env::var("BUILD_REVISION") {
        let rev = rev.trim().to_string();
        if !rev.is_empty() {
            return rev;
        }
    }

    let Some(head) = git(&["rev-parse", "HEAD"]) else {
        return "unknown".to_string();
    };

    // `--porcelain` prints nothing for a clean tree. Treat a failed status check
    // as clean rather than guessing dirty, so the marker means something.
    match git(&["status", "--porcelain"]) {
        Some(status) if !status.is_empty() => format!("{head}-dirty"),
        _ => head,
    }
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}
