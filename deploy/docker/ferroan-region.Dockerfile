# SPDX-License-Identifier: GPL-3.0-or-later
# Compiler tag is rust-toolchain.toml's channel. The toolchain file is not
# copied: it would download clippy and rustfmt, which this build does not run.
FROM rust:1.100.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY tools ./tools
COPY xtask ./xtask
RUN cargo build --locked --release -p ferroan-region -p ferroan-robust

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin app \
 && mkdir -p /app/config /app/data /app/logdata \
 && chown -R app:app /app
COPY --from=build --chown=app:app /src/target/release/ferroan-region /app/ferroan-region
USER app
WORKDIR /app
ENV LOGDIR=/app/logdata
ENTRYPOINT ["/app/ferroan-region"]
