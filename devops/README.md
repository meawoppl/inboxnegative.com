# devops/ — legacy deployment artifacts

> **Nothing in this directory is used by the current deployment.**

Both files below describe the AWS/ECS topology that was **retired during the
2026-08 GHCR migration**. They are kept so the old setup stays recoverable, not
because anything reads them.

| File | What it was | Status |
|---|---|---|
| `task-definition.json` | ECS task definition (`inboxnegative-task`), referencing the ECR image and the `inboxnegative-cluster` service | Dead — no ECS cluster exists |
| `nginx/inboxnegative.conf` | nginx vhost terminating TLS for `inboxnegative.com` | Superseded by Traefik on the deployment host |

Deployment today builds a release binary in CI and publishes an image to **GHCR**
(`ghcr.io/meawoppl/inboxnegative.com`) — see `.github/workflows/container.yml`.
The Infrastructure section of `CLAUDE.md` has the current picture, and marks which
claims are repo-verified versus reported.

Do not treat any account ID, ARN, IP address, or hostname in these files as live.

**If the ECS path is confirmed fully retired, this whole directory can be deleted**
(issue #4 raises this). It was retained rather than removed because that is a
judgement call about how much history to keep, not a mechanical cleanup.
