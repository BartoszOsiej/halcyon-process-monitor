# ── Stage 1: Build ──
FROM rustlang/rust:nightly-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release -p process-monitor

# ── Stage 2: Runtime ──
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/process-monitor /usr/local/bin/
RUN chmod +x /usr/local/bin/process-monitor
EXPOSE 0
ENTRYPOINT ["process-monitor"]
