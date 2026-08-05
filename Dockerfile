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

# The ZMQ publisher binds a relative `ipc://local_publisher_<pid>` socket, which
# is created in the working directory. Without this, cwd is `/`, which appuser
# cannot write -- the bind fails with EACCES and takes the SMTP task down. The
# old image ran as root, so `/` was writable and this never surfaced.
WORKDIR /home/appuser

# SMTP (2525) and HTTP (8080)
EXPOSE 2525 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/api/health || exit 1

CMD ["inboxnegative"]
