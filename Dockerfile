# ── Stage 1: Build ──
FROM rustlang/rust:nightly-slim AS builder
WORKDIR /src
COPY . .
RUN cd process-monitor && cargo build --release

# ── Stage 2: Runtime ──
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/process-monitor/target/release/process-monitor /usr/local/bin/
RUN chmod +x /usr/local/bin/process-monitor
EXPOSE 0
ENTRYPOINT ["process-monitor"]
