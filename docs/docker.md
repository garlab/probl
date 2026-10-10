# Container image

The image ([Dockerfile](../Dockerfile), published by [release.yml](../.github/workflows/release.yml) as `ghcr.io/garlab/probl`) is the `probl` command on Debian, built against glibc. This records why, so that the base images aren't swapped for Alpine or a static musl build without measuring again.

## Base images

- **Build:** `rust:<version>-slim-trixie`. **Run:** `debian:trixie-slim`. Both are pinned by digest, and Dependabot proposes updates. They share Debian trixie's glibc, which the binary links against.
- **glibc's allocator.** Probl allocates constantly, from every core when sampling. musl's allocator makes those threads wait on each other: see below.
- **A shell.** CI systems such as GitLab CI run scripts inside a job's image. Distroless and static images have no shell.
- **No login to pull.** Docker Hardened Images (`dhi.io`) need a Docker account, which the release workflow and Dependabot would then have to store.

The image is 32 MB uncompressed, of which the binary is 2.8 MB: it is built with the `dist` profile, which optimizes the whole program together ([build profiles](benchmarks.md#build-profiles)). The same command, built statically with musl on `scratch`, is 1.6 MB.

## glibc and musl, measured

Measured on 9 October 2026, with Probl 0.2.1 and Docker Desktop on an Apple Silicon Mac with 12 cores, running linux/arm64. The same commit was built in `rust:1.99.0-slim-trixie` (glibc, on `debian:trixie-slim`) and in `rust:1.99.0-alpine` (musl, static, on `scratch`). Each time is the best of 5 runs, alternating the two images, and includes about 0.2 s of container start.

| Model | glibc | musl |
|---|---|---|
| `05_snakes_and_ladders`, enumerated | 0.39 s | 0.43 s |
| `benches/blackjack_shoe`, enumerated | 0.20 s | 0.21 s |
| `08_signup_forecast`, sampled, 1 thread | 2.41 s | 2.75 s |
| `15_service_queue`, 50,000 runs, 1 thread | 5.47 s | 6.59 s |
| `08_signup_forecast`, sampled, 12 threads | 0.56 s | 12.96 s |
| `15_service_queue`, 200,000 runs, 12 threads | 2.87 s | 160.52 s |

On one thread, musl costs 7–20%. Sampling uses every core by default, and there musl's single allocator lock makes runs 23–56 times slower. A smaller image would need a static glibc build, or another allocator with musl, and new measurements.
