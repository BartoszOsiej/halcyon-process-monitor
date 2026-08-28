# Changelog

All notable changes to halcyon-process-monitor will be documented in this file.

## [0.6.0] - 2026-08-28

### Enterprise Maturity Program

**Level 1 — Supply Chain Security Foundation** ✅
- `cargo-deny` CI gate — blocks PRs with forbidden licenses and known advisories
- CycloneDX SBOM generation — JSON + XML SBOM attached to every release
- gitleaks secret scanning — detects hardcoded secrets in CI
- Dependency review — automated review of new dependencies on PRs
- Enhanced SECURITY.md — enterprise-grade VDP with SLA timelines

**Level 2 — Build Provenance & Signing** ✅
- SLSA Level 2 provenance — `slsa-framework/slsa-github-generator` for all releases
- Sigstore cosign keyless signing — OIDC-based binary signing
- GitHub artifact attestation — `gh attestation write` for supply chain integrity
- SHA-256 + SHA-512 checksums — cryptographically verifiable release integrity
- SBOM signing — Cosign signs CycloneDX SBOM alongside binaries
- Full release flow — build → sign → attest → publish with provenance

**Level 3 — Security Hardening & Audit** ✅
- `#![warn(missing_docs)]` crate-level lint enforced
- SAFETY comments documented on every `unsafe` block (6 blocks audited)
- `SecurityHeadersLayer` middleware — CSP, X-Frame-Options: DENY, nosniff, referrer-policy, permissions-policy
- Added `tower` dependency for axum middleware layer
- Fixed 2 pre-existing clippy warnings (unused variables in storage/mod.rs, tui.rs)
- Crate-level doc comment and function docs on public API

**Level 4 — Testing & Quality Gates** ✅
- Test suite expanded: 13 → **36 tests** (+23 new, 177% increase)
- Edge case tests: extract_extension (8 cases), cstr_to_string (5 variants)
- Shannon entropy tests: uniform (=0), high (>0.5), single char
- Monitor invariants: window eviction, threshold=0 disable, auto-kill action
- Empty state tests: top_files, extension_counts, rate_history, stats_sorted
- Init state tests: uptime, total_events, total_lost
- Clippy: 0 warnings with `-D warnings`

**New Files**
- `MATURITY.md` — 20-level enterprise maturity model
- `.github/workflows/supply-chain.yml` — Supply chain security CI
- `.github/workflows/build-provenance.yml` — Build provenance & signing

### Changed
- Upgraded `SECURITY.md` to enterprise-grade VDP with CVSS-based SLAs
- Added Hall of Fame section for security researchers
- Added SAFETY documentation to all unsafe blocks in `main.rs` and `monitor.rs`
- Added security headers middleware to `web.rs`
- Added 23 new unit tests in `monitor.rs`

## [0.5.0] - 2026-08-20

### Added
- Network tracepoints (connect/accept/sendto/recvfrom) with in-kernel sockaddr parsing
- Kill tracepoint with signal name resolution
- Filesystem tracepoints (mkdir/unlinkat/fchmodat)
- Process tree hierarchical view with PPID resolution
- Shannon entropy scoring for top files
- Network panel with Canvas traffic flow visualization
- Heatmap panel (process × extension matrix)
- Kafka event streaming with lz4 compression
- ClickHouse batch storage backend
- MemGraph process relationship graph
- Tauri desktop dashboard (React 19 + Recharts)
- Go web frontend
- C eBPF standalone variant

### Fixed
- LLVM `.text.unlikely` cold-path sections for bpf_probe_read_user
- Network tracepoint byte-by-byte sockaddr reading (avoided array types/slice operations)
- Kill tracepoint PID formatting with direct pointer arithmetic
- BPF codegen-units changed from 4 to 1 for reduced duplication

## [0.4.0] - 2025-08-01

### Added
- Criterion benchmarks (JSON serialization, event filtering, TUI render, atomic ops)
- Landing page with glassmorphism design
- Published to crates.io

### Changed
- Improved CI/CD pipeline with Ultra CI
- Enhanced TUI panels

## [0.3.0] - 2025-07-01

### Added
- eBPF process monitoring
- Real-time TUI dashboard
- Ransomware detection heuristics
- Network connection tracking
- Docker container support
- Web server mode (axum)
- Prometheus metrics

## [0.2.0] - 2025-03-01

### Added
- File operation tracking (open, read, write)
- Process tree visualization

## [0.1.0] - 2025-01-01

### Added
- Initial eBPF probe
- Basic process exec tracking
