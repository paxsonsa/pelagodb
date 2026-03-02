# syntax=docker/dockerfile:1

FROM rust:1.91 AS build
WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p pelago-operator --bin pelago-operator

FROM debian:bookworm-slim
RUN useradd --uid 10001 --create-home pelago
USER pelago
WORKDIR /home/pelago
COPY --from=build /workspace/target/release/pelago-operator /usr/local/bin/pelago-operator

ENTRYPOINT ["/usr/local/bin/pelago-operator"]
