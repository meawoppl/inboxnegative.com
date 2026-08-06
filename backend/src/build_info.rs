//! Build provenance reported by the running binary.
//!
//! The point is to answer "what is actually serving?" without inferring it from a
//! container image label. A label describes the image; this describes the process
//! that is running. Populated by `build.rs`.

/// Git revision this binary was built from. `"unknown"` when the build had no git
/// available, and suffixed `-dirty` when built from an unclean tree.
pub const REVISION: &str = env!("BUILD_REVISION");

/// Crate version from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One-line summary for the startup log. Deployment tooling gates on this by
/// reading logs, which needs no authenticated request -- unlike the HTTP endpoint.
pub fn summary() -> String {
    format!("inboxnegative {VERSION} revision {REVISION}")
}

/// Machine-readable form served at `/api/version`.
pub fn as_json() -> String {
    // Hand-rolled rather than via serde: both fields are compile-time constants
    // from our own build, so there is nothing here that needs escaping.
    format!(r#"{{"version":"{VERSION}","revision":"{REVISION}"}}"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_is_populated() {
        assert!(!REVISION.is_empty(), "build.rs must always set a revision");
    }

    /// A real revision is a 40-char hex SHA, optionally `-dirty`. Anything else
    /// must be exactly "unknown" -- a malformed value would be worse than an
    /// honest absence, since deployment tooling gates on this.
    #[test]
    fn revision_is_a_sha_or_unknown() {
        if REVISION == "unknown" {
            return;
        }
        let sha = REVISION.strip_suffix("-dirty").unwrap_or(REVISION);
        assert_eq!(sha.len(), 40, "expected a 40-char SHA, got {REVISION:?}");
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "expected hex, got {REVISION:?}"
        );
    }

    #[test]
    fn summary_names_version_and_revision() {
        let s = summary();
        assert!(s.contains(VERSION), "{s}");
        assert!(s.contains(REVISION), "{s}");
    }

    #[test]
    fn json_is_well_formed() {
        let json = as_json();
        assert_eq!(
            json,
            format!(r#"{{"version":"{VERSION}","revision":"{REVISION}"}}"#)
        );
        // Neither constant may contain a quote or backslash, or the hand-rolled
        // JSON above would be invalid.
        for field in [VERSION, REVISION] {
            assert!(!field.contains('"'), "{field:?} would break the JSON");
            assert!(!field.contains('\\'), "{field:?} would break the JSON");
        }
    }
}
