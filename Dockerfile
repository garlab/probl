# syntax=docker/dockerfile:1

# The `probl` command, in a small image:
#
#   docker run --rm -v "$PWD":/work ghcr.io/garlab/probl run model.probl
#
# .github/workflows/container.yml builds it for linux/amd64 and linux/arm64
# from each version tag. The base images are pinned by digest, and
# Dependabot proposes their updates.

FROM rust:1.99.0-slim-trixie@sha256:24e632c09342c20abf8312cf4f61430a911c01ed3a5e4c02b87292b1c39c5273 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# Without the debug information that the workspace's release profile keeps
# for profiling.
ENV CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p probl-cli \
    && cp target/release/probl /usr/local/bin/probl

FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
COPY --from=build /usr/local/bin/probl /usr/local/bin/probl
# It only reads what it's given: run it without privileges, in the folder
# the models are mounted at.
RUN useradd --uid 10001 --create-home probl
USER probl
WORKDIR /work
ENTRYPOINT ["probl"]
CMD ["--help"]
