# Multi-stage build for PelagoDB server
FROM rust:1.82 AS builder

# Install FDB client libraries
RUN curl -L https://github.com/apple/foundationdb/releases/download/7.3.27/foundationdb-clients_7.3.27-1_amd64.deb -o fdb.deb \
    && dpkg -i fdb.deb \
    && rm fdb.deb

WORKDIR /app
COPY . .
RUN cargo build --release --bin pelago-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/lib/libfdb_c.so /usr/lib/
COPY --from=builder /app/target/release/pelago-server /usr/local/bin/
EXPOSE 50051
CMD ["pelago-server"]
