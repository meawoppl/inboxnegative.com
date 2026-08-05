# Claude Notes and Preferences

## Commit Guidelines
- Do not include Claude attribution in commit messages
- Use clear, descriptive commit messages that explain what was changed and why
- Include bullet points for complex changes
- Do not include the "🤖 Generated with Claude Code" footer
- Do not include "Co-Authored-By: Claude" in commits

## Code Style
- Follow Rust idiomatic style
- Use Cargo fmt for formatting

## Dependency Management
- Always use `cargo add` to add dependencies (not manual Cargo.toml editing)
- This ensures we get the newest compatible versions
- `Cargo.lock` is committed (this is an application) — CI builds with `--locked`
- Run `cargo machete` periodically to catch declared-but-unused dependencies
- TLS is **rustls-only**: `reqwest` uses `default-features = false` with `rustls-tls`.
  Do not reintroduce `native-tls`/`openssl`.

## Common Commands
```bash
# Add a dependency
cargo add <package-name>

# Format code
cargo fmt

# Check for errors
cargo check

# Build the project
cargo build

# Run tests
cargo test

# Check formatting (CI gate)
cargo fmt --all --check

# Lint (CI gate; warnings are denied)
RUSTFLAGS="-Dwarnings" cargo clippy -p inboxnegative -p shared --all-targets --locked
RUSTFLAGS="-Dwarnings" cargo clippy -p inboxnegativefrontend --target wasm32-unknown-unknown --locked

# Security advisories
cargo audit

# Unused dependencies
cargo machete
```

## Continuous Integration
CI (`.github/workflows/rust.yml`) runs these jobs on every push/PR to `main`:

| Job | Required? | What it checks |
|-----|-----------|----------------|
| Rustfmt | yes | `cargo fmt --all --check` |
| Clippy | yes | `clippy` under `-Dwarnings`, split by target (host crates + wasm frontend) |
| Build & Test | yes | backend build, `trunk build` frontend, `cargo test` |
| Security Audit | no (advisory) | `cargo audit` |

Notes:
- Clippy is split per target because the wasm frontend can't be linted for the host
  target. Run both invocations locally before pushing.
- The Security Audit job is `continue-on-error` (advisory only). Reviewed advisories
  that don't apply to our config are ignored in `.cargo/audit.toml` with a rationale.
- All cargo commands run with `--locked`, so `Cargo.lock` must be committed and current.

## Security

### Rotated secrets (do not flag as live leaks)
- The historical Google OAuth client secret `GOCSPX-…Vqk7ALY` that appears in old
  git history (formerly hardcoded in `backend/src/http.rs` and committed in
  `backend/.env`) has been **rotated and is dead**. It is no longer valid and is
  not a live credential — secret scanners can ignore it. It was removed from the
  working tree (hardcoded fallback dropped, `backend/.env` untracked); history
  still contains it but it carries no risk.

## Database Information
- The application uses PostgreSQL with Diesel ORM for stats storage
- Database setup requires PostgreSQL client and development libraries
- Migrations are embedded in the binary and applied on startup (`backend/migrations`)
- The database is **required**. If the pool cannot be created the process logs the
  reason and exits; there is no file-based fallback
- Local development reads `DATABASE_URL` from `backend/.env` (gitignored). Production
  runs a self-hosted `postgres:17-alpine` container, **not** Neon — the Neon project
  was retired during the 2026-08 GHCR migration

## Infrastructure

> **Last verified against the repository: 2026-08-05.** Claims below are split by
> how they were checked. Anything marked *(repo-verified)* was read out of a file
> in this tree. Anything marked *(reported)* comes from issue #4 and the 2026-08
> migration notes and was **not** independently confirmed against a running host —
> treat it as the best available account, not as ground truth, and re-check before
> relying on it for anything destructive.

### Build and image *(repo-verified)*
- CI builds a release binary and pushes an image to **GHCR**, not ECR:
  `ghcr.io/meawoppl/inboxnegative.com`, tagged `latest` and the commit SHA
  (`.github/workflows/container.yml`)
- On pull requests the image is built but **not** pushed; only pushes to `main` publish
- The frontend is built with `trunk` *before* the backend, because rust-embed embeds
  `frontend/dist` into the release binary. The image is the single self-contained
  binary plus runtime libs — no assets on disk
- Runs as `appuser` (uid 1001) with `WORKDIR /home/appuser`
- The Dockerfile **has** a `HEALTHCHECK` hitting `/api/health` — it has been there since
  #69/#71. Do not describe it as missing; check the file rather than a deployed image

### Deployment *(reported)*
- GHCR image pulled on a single host, run under Docker Compose with Watchtower for
  image updates, fronted by Traefik
- **There is no ECS, ECR, NLB, or EC2 deployment.** `./push-to-ecr.sh` is referenced by
  older docs but does not exist in this repo

### SMTP port architecture *(repo-verified in part)*
- The app listens on **2525**, not 25, to avoid privileged-port issues in containers
  (`EXPOSE 2525 8080` in the Dockerfile) *(repo-verified)*
- Port 25 → 2525 translation is handled by the fronting proxy on the host *(reported)*

### Legacy files, retained but not live
- `devops/task-definition.json` — ECS task definition. Dead; the ECS path is retired
- `devops/nginx/inboxnegative.conf` — nginx vhost, superseded by Traefik *(reported)*
- `docs/aws-deployment.md` — the full AWS/ECS runbook, kept for history only

These are left in place rather than deleted so the old topology stays recoverable, but
**nothing in them describes how the service is deployed today.**

### Why this section carries provenance markers
`CLAUDE.md` is the first file agents and new contributors read, so a stale line here is
not a documentation nicety — it becomes a confidently wrong premise that gets acted on.
This section was wrong about production twice during the 2026-08 migration: it named
Neon as the live database (causing a rotated credential to be assessed as live), and it
described an ECS deployment that no longer existed. The recurring failure mode was
checking a proxy and reporting it as the property — an old image for current source,
filenames for content. Marking each claim with how it was checked is meant to make that
substitution visible instead of invisible.