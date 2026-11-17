# syntax=docker/dockerfile:1

# Debian and glibc, not Alpine or musl: see docs/docker.md.
FROM rust:1.99.0-slim-trixie@sha256:24e632c09342c20abf8312cf4f61430a911c01ed3a5e4c02b87292b1c39c5273 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# The workspace's `dist` profile: see Cargo.toml. `target` is a cache mount,
# so the binary is copied out of it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --profile dist --locked -p probl-cli \
    && cp target/dist/probl /usr/local/bin/probl

FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
COPY --from=build /usr/local/bin/probl /usr/local/bin/probl
RUN useradd --uid 10001 --create-home probl
USER probl
WORKDIR /work
ENTRYPOINT ["probl"]
CMD ["--help"]
