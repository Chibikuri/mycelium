FROM rust:1.83-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src/ src/

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    git \
    grep \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/mycelium /usr/local/bin/mycelium

RUN useradd -m -s /bin/bash mycelium
USER mycelium

WORKDIR /home/mycelium

EXPOSE 3000

ENTRYPOINT ["mycelium"]
