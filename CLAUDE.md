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
- The application uses PostgreSQL with Diesel ORM for stats storage
- Database setup requires PostgreSQL client and development libraries
- Migrations are embedded in the binary and applied on startup (`backend/migrations`)
- The database is **required**. If the pool cannot be created the process logs the
  reason and exits; there is no file-based fallback
- Local development reads `DATABASE_URL` from `backend/.env` (gitignored). Production
  runs a self-hosted `postgres:17-alpine` container, **not** Neon — the Neon project
  was retired during the 2026-08 GHCR migration and has since been deleted (issue #3)
  *(host-verified 2026-08-06)*

## Hash salts — do not change these values

`EMAIL_SALT` and `DELETED_SALT` are **required** environment variables. `salt::init()`
refuses to boot without them, by design: booting with different salts would silently
produce different addresses and stats keys, orphaning every existing record rather
than failing.

**Their values must never change.** `EMAIL_SALT` derives every public burner address
ever handed out, and `DELETED_SALT` is the primary key into `email_stats` — 1411 rows
as of the 2026-08 cutover. Rotating either is not a credential rotation; it is a
silent data loss event, and nothing in the code will complain.

Sourced from 1Password at
`op://meawoppl-infrastructure/inboxnegative/{email_salt,deleted_salt}` and injected
into `/opt/services/env/inboxnegative.env` on the host *(host-verified 2026-08-06)*.
Ordering matters on deploy: **the env must be live before the image that needs it**,
or the container will not start.

Defined in `backend/src/salt.rs` (`EMAIL_SALT_VAR`, `DELETED_SALT_VAR`). If you change
how salts are read, preserve both the fail-fast and the exact byte sequence fed to the
hash — `backend/src/deleted.rs::tests::test_hash_with_salt_is_salt_then_email` pins the
ordering as salt-then-email so an accidental swap fails loudly instead of silently
re-keying every record.

## Infrastructure

> **Last verified: 2026-08-06.** Claims below are split by how they were checked.
> *(repo-verified)* was read out of a file in this tree. *(host-verified)* was
> inspected directly on the deployment host on 2026-08-06 by the
> meawoppl-infrastructure session (`72f72c15`), which has SSH access; this repo
> cannot confirm those independently, so they are as good as that session's
> observation and no better. *(reported)* is second-hand and confirmed by nobody —
> re-check before relying on it for anything destructive.
>
> Nothing in this repo describes the deployment host: there is no compose file,
> and no Traefik or Watchtower reference anywhere in the tree. Every host fact
> below therefore has to be attributed rather than asserted.

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

### Deployment *(host-verified 2026-08-06)*
A single EC2 host, `44.224.90.181`, with everything under `/opt/services`.
**`/opt/services` is not a git repository — files are copied to the host manually**,
so there is no version control on the deployment config and no way to diff it from
here. Treat any host config quoted in this file as a snapshot, not a source of truth.

| Thing | Where |
|---|---|
| Compose file | `/opt/services/docker-compose.yml` |
| Traefik dynamic config | `/opt/services/traefik/dynamic/mail.yml` (file provider; TCP routers for the `smtp` and `submission` entrypoints, both → `inboxnegative:2525`) |
| Traefik static config | command-line args in the traefik service block of the compose file — there is no separate `traefik.yml` |
| Image in use | `ghcr.io/meawoppl/inboxnegative.com:latest` |
| Database | self-hosted `postgres:17-alpine` on the same box, reachable on the docker network as `host=postgres`, database `inboxnegative` |

`inboxnegative` is gated on the database with
`depends_on: postgres: condition: service_healthy`. This matters because pool
failure is fatal (see Database Information above) — without the gate, a slow
Postgres start means the app exits instead of waiting. **The gate does not cover
host reboots:** Docker's restart policy starts containers without compose's
dependency ordering, so `restart: unless-stopped` is what recovers that case, by
retrying until Postgres is up.

**There is no ECS, ECR, NLB, or EC2-with-ECS-agent deployment.** `./push-to-ecr.sh`
is referenced by older docs and does not exist in this repo.

### Watchtower *(host-verified 2026-08-06)*
Auto-deploys `:latest` unattended. `WATCHTOWER_POLL_INTERVAL=300`,
`WATCHTOWER_LABEL_ENABLE=true`, `WATCHTOWER_CLEANUP=true`,
`WATCHTOWER_INCLUDE_RESTARTING=true`. Observed lag from merge to live is about six
minutes. Enrolment is **opt-in** via the label
`com.centurylinklabs.watchtower.enable=true`; five containers are enrolled
(`inboxnegative`, `agent-proxy`, `agentive-inversion`, `pastebom`, `rps-arena`).

Two failure modes worth knowing before trusting it:
- `LABEL_ENABLE=true` means an **unlabelled container is silently never scanned**.
  That is half of why this service ran six months stale — CI published to GHCR while
  the container carried no Watchtower label.
- Watchtower has **no health gating and no notifier configured**. It will happily
  deploy a crash-looping image and tell nobody. Separate host alerting covers this.

Practical consequence: **anything merged to `main` is live within ~6 minutes with no
human in the loop.** Behaviour changes that affect startup — the fatal database
pool, the ZMQ socket path — reach production on that timer.

### SMTP port architecture
- The app listens on **2525**, not 25, to avoid privileged-port issues in containers
  (`EXPOSE 2525 8080` in the Dockerfile) *(repo-verified)*
- Host port 25 → Traefik → `inboxnegative:2525`. Answering from the box with
  `220 inboxnegative.com SMTP Server DTF` *(host-verified 2026-08-06)*
- **This has not been verified from off-host recently.** The last external check was
  during the 2026-08-04 cutover. A 2026-08-06 attempt was inconclusive because
  outbound 25 was blocked at the checking end, not because the service failed.
  Reaching the SMTP listener from the host chain and reaching it from the internet
  are different claims; only the former is currently evidenced.

### Legacy files, retained but not live
- `devops/task-definition.json` — ECS task definition. Dead; the ECS path is retired
- `devops/nginx/inboxnegative.conf` — nginx vhost. Superseded: Traefik does the
  routing *(host-verified 2026-08-06)*
- `docs/aws-deployment.md` — the full AWS/ECS runbook, kept for history only

These are left in place rather than deleted so the old topology stays recoverable, but
**nothing in them describes how the service is deployed today.** Note both `devops/`
files are also reproduced verbatim inside `docs/aws-deployment.md`
(`task-definition.json` at its ECS Deployment section, the nginx vhost including the
HTTPS block at its SSL section), so deleting the directory would lose nothing.

### Why this section carries provenance markers
`CLAUDE.md` is the first file agents and new contributors read, so a stale line here is
not a documentation nicety — it becomes a confidently wrong premise that gets acted on.
This section was wrong about production twice during the 2026-08 migration: it named
Neon as the live database (causing a rotated credential to be assessed as live), and it
described an ECS deployment that no longer existed. The recurring failure mode was
checking a proxy and reporting it as the property — an old image for current source,
filenames for content. Marking each claim with how it was checked is meant to make that
substitution visible instead of invisible.

That failure mode is not hypothetical here. `.cargo/audit.toml` once suppressed
RUSTSEC-2024-0421 on the stated grounds that "idna is not in the active dependency
tree" — while `idna` was in fact reachable on normal edges via `url` from `ammonia`,
`reqwest`, `tower-http`, and `shared` directly. A live advisory sat silently
suppressed behind a premise that was false when written, because nothing re-checks a
rationale once it is in the file. Hence: state the *condition*, and say how you
checked it.

The tiers are ordered by how easily a claim can be re-derived, not by confidence.
*(repo-verified)* can be re-checked by anyone reading this tree, so it decays
loudly — the file it cites will contradict it. *(host-verified)* cannot be
re-checked from here at all and decays **silently**: the host can change with
nothing in this repo registering it. Carry the date and the session forward when
you touch those lines, and prefer re-verifying to copying.