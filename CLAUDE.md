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
Two workflows run on every push/PR to `main`. **All seven jobs are required** — they
are enforced as required status checks by branch protection on `main`, so a failure
blocks merge rather than merely reporting.

From `.github/workflows/rust.yml`:

| Job | What it checks |
|-----|----------------|
| Rustfmt | `cargo fmt --all --check` |
| Clippy | `clippy` under `-Dwarnings`, split by target (host crates + wasm frontend) |
| Build & Test | `trunk build` frontend, backend build, `cargo test` |
| Security Audit | `cargo audit` |
| Migration Names | `./scripts/check-migration-names.sh` |

From `.github/workflows/container.yml`:

| Job | What it checks |
|-----|----------------|
| Build Release Artifacts | release frontend (`trunk build --release`) + release backend |
| Build Container | `docker build` of the runtime image (pushes to GHCR only on `main`) |

Notes:
- Clippy is split per target because the wasm frontend can't be linted for the host
  target. Run both invocations locally before pushing.
- The Clippy job creates an empty `frontend/dist` rather than running `trunk`:
  rust-embed needs the directory to exist at compile time, but clippy only
  type-checks and never reads the assets. Build & Test does the real `trunk build`.
- The Security Audit job is **blocking** (it was `continue-on-error` while an
  advisory backlog existed; that backlog is cleared). The `ignore` list in
  `.cargo/audit.toml` is deliberately **empty** — every advisory is fixed outright
  rather than waived. Prefer upgrading; only add an entry when no patched version
  exists, and state the *condition* that makes it non-applicable, not just the
  conclusion, so it fails loudly when that condition stops holding.
- `cargo audit` still reports two `unmaintained` warnings (`bincode`,
  `proc-macro-error`). Both are transitive through `yew` 0.21 and have no patched
  version; see issue #9. These are informational, not vulnerabilities.
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
- The application now uses PostgreSQL with Diesel ORM for stats storage
- Database setup requires PostgreSQL client and development libraries
- Connection string format: postgresql://username:password@hostname/dbname?sslmode=require
- The Neon DB is configured in backend/.env

## AWS Infrastructure

### SMTP Port Architecture
- **App listens on port 2525** (not 25, to avoid privileged port issues in containers)
- **NLB `inboxnull`** handles port translation: public port 25 → target port 2525
- NLB DNS: `inboxnull-5879fc3a6213a0c5.elb.us-west-2.amazonaws.com`
- Target group: `inboxnull-2525` (forwards to ECS tasks on port 2525)

### DNS Configuration for Email Delivery
- MX record: `mail.inboxnegative.com`
- **IMPORTANT**: For Gmail/external SMTP delivery to work, `mail.inboxnegative.com` must point to the NLB (not directly to EC2)
- If mail isn't being received from Gmail, check that DNS points to NLB
- Direct EC2 IP (18.237.84.151) only has port 2525 open, not port 25

### ECS Deployment
- Cluster: `inboxnegative-cluster`
- Service: `inboxnegative-service`
- Task definition: `inboxnegative-task`
- ECR repo: `877983347039.dkr.ecr.us-west-2.amazonaws.com/inboxnegative`
- Deploy script: `./push-to-ecr.sh` (builds, pushes, registers task, updates service)