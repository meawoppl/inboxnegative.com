# Slim runtime image. The binary is built in CI and placed under build-output/
# before `docker build` (see .github/workflows/container.yml). The frontend
# assets and database migrations are embedded in the binary, so the image is
# just the single binary plus runtime libs.
#
# Base must be <= the glibc of the build environment. CI builds on ubuntu-22.04
# (glibc 2.35); debian:bookworm (glibc 2.36) is newer, so the binary runs.
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libpq5 curl && \
    rm -rf /var/lib/apt/lists/*

ENV RUST_BACKTRACE=1 \
    RUST_LOG=info

# Prebuilt, self-contained binary (frontend assets + migrations are embedded).
COPY build-output/inboxnegative /usr/local/bin/inboxnegative

RUN useradd -m -u 1001 -s /bin/bash appuser
USER appuser

# Historically this was load-bearing: the ZMQ publisher bound a *relative*
# `ipc://local_publisher_<pid>` socket, so it landed in cwd. With `USER appuser`
# and no WORKDIR, cwd was `/`, the bind failed EACCES, and the SMTP task took the
# process down. The socket path is now absolute (ZMQ_SOCKET_DIR, else
# XDG_RUNTIME_DIR, else /tmp), so that specific failure can no longer happen.
#
# Kept anyway: a writable cwd owned by the running user is a sane default, and
# nothing should silently depend on cwd being `/` either.
WORKDIR /home/appuser

# SMTP (2525) and HTTP (8080)
EXPOSE 2525 8080

# Traefik's docker provider will not route to a container that is not yet healthy,
# so this interval IS the user-visible outage window on every deploy -- not Traefik
# lag. Docker schedules the first probe at `interval`, regardless of `start_period`,
# so a 30s interval meant ~30s of 404s even though the app was serving within
# seconds. Measured on the host 2026-08-07: container started 00:40:45.711, first
# probe 00:41:15.722 (+30.011s), passed on the first attempt.
#
# `interval x retries` is also time-to-unhealthy, and Traefik gates in BOTH
# directions -- so dropping the interval alone would have made the service six
# times twitchier about a transient blip, trading a predictable window at deploy
# time for unpredictable ones at arbitrary times. `retries` is raised to hold that
# product at 90s: 5 x 18 == 30 x 3.
#
# 90s of tolerance is safe here because /api/health is pure liveness -- it takes no
# state, touches no database, and returns a static body. If it ever starts checking
# the pool, this becomes a tolerance for a Postgres hiccup and should be revisited.
HEALTHCHECK --interval=5s --timeout=3s --start-period=5s --retries=18 \
    CMD curl -f http://localhost:8080/api/health || exit 1

CMD ["inboxnegative"]
