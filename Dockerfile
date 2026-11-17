# syntax=docker/dockerfile:1

# Debian and glibc, not Alpine or musl: see docs/docker.md.
FROM rust:1.99.1-slim-trixie@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# The workspace's `dist` profile: see Cargo.toml. `target` is a cache mount,
# so the binary is copied out of it.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --profile dist --locked -p probl-cli \
    && cp target/dist/probl /usr/local/bin/probl

FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
COPY --from=build /usr/local/bin/probl /usr/local/bin/probl
RUN useradd --uid 10001 --create-home probl
USER probl
WORKDIR /work
ENTRYPOINT ["probl"]
CMD ["--help"]
