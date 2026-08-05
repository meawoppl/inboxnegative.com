# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Security
- Resolved the entire `cargo audit` backlog: **19 vulnerabilities → 0**.
  - `ammonia` 4.0 → 4.1.2 (RUSTSEC-2025-0071, mutation XSS in the HTML sanitizer
    used for untrusted email content).
  - `bytes` 1.7 → 1.12 (RUSTSEC-2026-0007, integer overflow in `BytesMut::reserve`).
  - `diesel` 2.2 → 2.3 (RUSTSEC-2026-0136, COPY injection) plus a required `syn` bump.
  - TLS stack: `openssl`, `rustls`, `rustls-webpki`, `ring` updated to patched versions.
  - `crossbeam-channel`, `time`, and yanked transitives updated.
- Switched to **rustls-only** TLS: `reqwest` now uses `default-features = false` with
  `rustls-tls`, removing `openssl`/`native-tls` from the build entirely.

### Added
- CI gates (`.github/workflows/rust.yml`): Rustfmt, Clippy (`-Dwarnings`, split by
  target), and a Security Audit job (advisory). `main` requires fmt/clippy/build+test.
- `.cargo/audit.toml` documenting advisories reviewed as not applicable to this app.
- `Cargo.lock` is now committed, enabling reproducible `--locked` builds in CI.

### Removed
- 18 unused dependencies pruned (`cargo machete` now reports none):
  `lettre`, `native-tls`, `jsonwebtoken`, `futures`, `rand`, `url`, `uuid`, `anyhow`,
  `cookie`, `lazy_static`, `js-sys`, `serde` (frontend), `urlencoding`. Several of
  these were the sole source of vulnerable transitive crates (`openssl`, `ring 0.16`).
