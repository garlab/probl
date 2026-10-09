# syntax=docker/dockerfile:1

# Debian and glibc, not Alpine or musl: see docs/docker.md.
FROM rust:1.99.0-slim-trixie@sha256:24e632c09342c20abf8312cf4f61430a911c01ed3a5e4c02b87292b1c39c5273 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# The workspace's release profile keeps debug information, for profiling.
ENV CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols
# `target` is a cache mount, so the binary is copied out of it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p probl-cli \
    && cp target/release/probl /usr/local/bin/probl

FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
COPY --from=build /usr/local/bin/probl /usr/local/bin/probl
RUN useradd --uid 10001 --create-home probl
USER probl
WORKDIR /work
ENTRYPOINT ["probl"]
CMD ["--help"]
