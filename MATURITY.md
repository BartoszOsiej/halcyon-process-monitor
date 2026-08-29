# Talus — Enterprise Maturity Model

> Corporate-grade security agent. From open-source prototype to Fortune 500 ready.

## Maturity Levels Overview

```
Level  0 ▓░░░░░░░░░░░░░░░░░░░░░░ Open Source Prototype
Level  1 ▓▓░░░░░░░░░░░░░░░░░░░░░ Supply Chain Security Foundation
Level  2 ▓▓▓░░░░░░░░░░░░░░░░░░░░ Build Provenance & Signing
Level  3 ▓▓▓▓░░░░░░░░░░░░░░░░░░░ Security Hardening & Audit
Level  4 ▓▓▓▓▓░░░░░░░░░░░░░░░░░░ Testing & Quality Gates ✅ CURRENT
Level  5 ▓▓▓▓▓▓░░░░░░░░░░░░░░░░░ Observability & Incident Response
Level  6 ▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░ Documentation & Knowledge Mgmt
Level  7 ▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░ Compliance Framework (SOC2/ISO27001)
Level  8 ▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░ Access Control & Secrets Management
Level  9 ▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░ Container & Image Security
Level 10 ▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░ Network Security & mTLS
Level 11 ▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░ Data Protection & Encryption
Level 12 ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░ Disaster Recovery & HA
Level 13 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░ Change Management & Rollback
Level 14 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░ Performance & Scalability
Level 15 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░ Monitoring, SLIs & SLOs
Level 16 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░ Business Continuity Planning
Level 17 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░ Third-Party Risk Management
Level 18 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░ Audit & Compliance Automation
Level 19 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░ Zero Trust Architecture
Level 20 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░ Enterprise Grade (SOC2 Type II + ISO27001)
```

---

## Level 0 — Open Source Prototype ✅ COMPLETE

| Area | Status |
|---|---|
| CI/CD | GitHub Actions (clippy, fmt, test, eBPF build) |
| Security scanning | CodeQL (Rust), OpenSSF Scorecard |
| Dependency mgmt | Dependabot (weekly), cargo-deny config |
| Fuzzing | cargo-fuzz harness |
| Testing | 13 unit tests, 10 tracepoints verified |
| Docs | README, ARCHITECTURE.md, SECURITY.md (basic) |
| Release | Tag-based, no signing |
| Containers | Dockerfile, GHCR publish |

---

## Level 1 — Supply Chain Security Foundation ✅ IMPLEMENTED

| Area | Implementation |
|---|---|
| **License compliance** | `cargo-deny` CI gate — blocks PRs with forbidden licenses |
| **Advisory audit** | `cargo-deny` advisory check — blocks on known vulnerabilities |
| **SBOM generation** | `cargo-cyclonedx` → CycloneDX XML SBOM attached to every release |
| **Dependency pinning** | Cargo.lock committed, dependabot with cooldown |
| **Security policy** | Enhanced SECURITY.md with VDP, scope, severity classification |
| **Provenance doc** | MATURITY.md — this file, tracks maturity progression |

**CI Gate:** `supply-chain.yml` runs on every PR and push to master.

---

## Level 2 — Build Provenance & Signing ✅ IMPLEMENTED

| Area | Implementation |
|---|---|
| **SLSA provenance** | `slsa-framework/slsa-github-generator` — Level 2 provenance |
| **Binary signing** | Sigstore cosign — keyless OIDC signing of release artifacts |
| **Build attestation** | `gh attestation verify` — GitHub artifact attestation |
| **Checksums** | SHA-256 + SHA-512 checksums for all release binaries |
| **Container signing** | Cosign signs OCI images pushed to GHCR |
| **Reproducible build** | Hash pinning in CI, deterministic output verification |

**Release flow:** Tag `v*` → build → sign → attest → publish with SBOM + provenance.

---

## Level 3 — Security Hardening & Audit ✅ IMPLEMENTED

| Area | Implementation |
|---|---|
| **SAFETY docs** | `#![warn(missing_docs)]` crate-level lint; SAFETY comments on all unsafe blocks |
| **Security headers** | `SecurityHeadersLayer` — CSP, X-Frame-Options: DENY, nosniff, referrer-policy, permissions-policy |
| **Unsafe audit** | Documented every `unsafe` block: geteuid, getuid, getpwuid_r, signal, kill, read_unaligned |
| **Clippy clean** | 0 warnings with `-D warnings` — fixed pre-existing unused variable warnings |
| **Crate docs** | Module-level doc comment on `main.rs` describing the agent's purpose |
| **Fn docs** | Documentation on `Monitor::start`, `kill_process`, `spawn_reader`, `passwd_dir` |

**Files changed:** `main.rs`, `monitor.rs`, `web.rs`, `Cargo.toml` (added `tower` dep)

---

## Level 4 — Testing & Quality Gates ✅ IMPLEMENTED

| Area | Implementation |
|---|---|
| **Test count** | 13 → **36 tests** (+23 new, 177% increase) |
| **Edge cases** | `extract_extension` 8 boundary cases, `cstr_to_string` 5 variants |
| **Entropy tests** | Uniform string (=0), high entropy (>0.5), single char |
| **Monitor invariants** | Window eviction, threshold=0 disable, auto-kill action, orphan PID |
| **Empty state tests** | top_files, extension_counts, rate_history, stats_sorted, flatten_tree |
| **Init state tests** | uptime near zero, total_events=0, total_lost=0 |
| **Clippy** | 0 warnings with `-D warnings` (fixed 2 pre-existing unused vars) |

**Test categories:** Edge cases · Property-like invariants · State transitions · Init state

**Remaining for full Level 4:** cargo-tarpaulin coverage, proptest integration, cargo-mutants

---

## Level 5 — Observability & Incident Response 🔜 NEXT

| Area | Target |
|---|---|
| **Structured logging** | `tracing` + `tracing-subscriber` with JSON output |
| **Distributed tracing** | OpenTelemetry spans for event pipeline stages |
| **Metrics enrichment** | Prometheus histograms for latency percentiles (p50/p95/p99) |
| **Alerting rules** | Prometheus AlertManager rules for agent health |
| **Incident response** | Documented IR plan with escalation matrix |
| **Post-mortem template** | Blameless post-mortem template in docs/ |

---

## Level 6 — Documentation & Knowledge Management

| Area | Target |
|---|---|
| **API documentation** | `rustdoc` with `#![warn(missing_docs)]` |
| **ADRs** | Architecture Decision Records in docs/adr/ |
| **Runbooks** | Operational runbooks for common scenarios |
| **Onboarding guide** | New developer onboarding checklist |
| **Diagram updates** | Mermaid diagrams auto-generated from code |
| **Changelog automation** | `git-cliff` or `cargo-changelog` from conventional commits |

---

## Level 7 — Compliance Framework

| Area | Target |
|---|---|
| **SOC2 Type I readiness** | Map controls to SOC2 trust service criteria |
| **ISO 27001 alignment** | Statement of Applicability mapping |
| **CIS Benchmark** | Linux hardening benchmark compliance |
| **NIST CSF** | NIST Cybersecurity Framework alignment |
| **Compliance evidence** | Automated evidence collection for audits |
| **Policy documents** | Acceptable use, data handling, access control policies |

---

## Level 8 — Access Control & Secrets Management

| Area | Target |
|---|---|
| **RBAC** | Role-based access for web dashboard and API |
| **API authentication** | JWT/OAuth2 for REST API endpoints |
| **Secret rotation** | Automated secret rotation policy |
| **Vault integration** | HashiCorp Vault for runtime secrets |
| **Audit logging** | Immutable audit trail for all admin actions |
| **SSO integration** | SAML/OIDC for enterprise SSO |

---

## Level 9 — Container & Image Security

| Area | Target |
|---|---|
| **Minimal base** | Distroless or scratch-based container image |
| **Image scanning** | Trivy + Grype in CI, block on critical CVEs |
| **Image signing** | Cosign signature for all published images |
| **SBOM in OCI** | Attach SBOM as OCI artifact to image manifest |
| **Network policies** | Kubernetes NetworkPolicy manifests |
| **Pod security** | Restricted Pod Security Standards |

---

## Level 10 — Network Security & mTLS

| Area | Target |
|---|---|
| **TLS everywhere** | All endpoints encrypted (REST, WebSocket, gRPC) |
| **mTLS** | Mutual TLS for agent-to-backend communication |
| **Certificate mgmt** | cert-manager integration for K8s |
| **Network segmentation** | Defined network zones and microsegmentation |
| **Egress control** | Allowlist-based egress for agent communication |
| **DNS security** | DNS-over-HTTPS, DNSSEC validation |

---

## Level 11 — Data Protection & Encryption

| Area | Target |
|---|---|
| **Encryption at rest** | Encrypted event storage (ClickHouse, file output) |
| **PII handling** | PII detection and redaction in event streams |
| **Data classification** | Data sensitivity labeling system |
| **Key management** | KMS integration for encryption keys |
| **Retention policy** | Automated data retention and secure deletion |
| **Right to erasure** | GDPR-compliant data deletion capability |

---

## Level 12 — Disaster Recovery & High Availability

| Area | Target |
|---|---|
| **Multi-region** | Multi-AZ / multi-region deployment capability |
| **Failover** | Automatic failover with health checks |
| **Backup strategy** | Automated, encrypted, tested backups |
| **RTO/RPO** | Defined Recovery Time/Point Objectives |
| **Chaos engineering** | Controlled failure injection testing |
| **Runbook automation** | Automated DR playbooks |

---

## Level 13 — Change Management & Rollback

| Area | Target |
|---|---|
| **Deployment stages** | Dev → Staging → Canary → Production |
| **Canary releases** | Gradual rollout with automatic rollback |
| **Feature flags** | LaunchDarkly / Unleash integration |
| **Rollback procedure** | Documented, tested rollback for every release |
| **Approval gates** | Required reviews and approvals for production changes |
| **Change freeze** | Defined change freeze periods |

---

## Level 14 — Performance & Scalability

| Area | Target |
|---|---|
| **Load testing** | k6 / Locust load test suite |
| **Capacity planning** | Resource usage forecasting models |
| **Auto-scaling** | Horizontal Pod Autoscaler for K8s deployment |
| **Performance budgets** | Defined latency and throughput budgets |
| **Resource limits** | CPU/memory limits with OOM protection |
| **Benchmarking suite** | Automated performance regression detection |

---

## Level 15 — Monitoring, SLIs & SLOs

| Area | Target |
|---|---|
| **SLIs defined** | Availability, latency, throughput indicators |
| **SLO targets** | 99.9% availability, <100ms p99 latency |
| **Error budgets** | Error budget consumption tracking |
| **On-call rotation** | PagerDuty / OpsGenie integration |
| **Escalation matrix** | Tiered escalation with time-based triggers |
| **Dashboard suite** | Grafana dashboards for all SLIs |

---

## Level 16 — Business Continuity Planning

| Area | Target |
|---|---|
| **BCP document** | Formal business continuity plan |
| **Communication plan** | Stakeholder notification procedures |
| **Alternate sites** | Warm/hot standby capability |
| **Tabletop exercises** | Quarterly BCP simulation drills |
| **Vendor BCP** | Third-party BCP verification |
| **Recovery testing** | Annual full recovery test |

---

## Level 17 — Third-Party Risk Management

| Area | Target |
|---|---|
| **Vendor assessment** | Security questionnaire for all dependencies |
| **Open source policy** | Formal OSS adoption and review process |
| **Supply chain attestation** | SLSA Level 3+ for all dependencies |
| **Dependency review** | Automated dependency review on PR |
| **License compliance** | Full license audit with exception tracking |
| **SBOM distribution** | SBOM shared with customers and regulators |

---

## Level 18 — Audit & Compliance Automation

| Area | Target |
|---|---|
| **Continuous compliance** | Automated compliance scanning (daily) |
| **Audit trails** | Immutable, tamper-evident audit logs |
| **Evidence collection** | Automated evidence gathering for auditors |
| **Compliance dashboard** | Real-time compliance status visibility |
| **Policy-as-code** | OPA / Rego policies enforced in CI |
| **Regulatory tracking** | Automated regulatory change monitoring |

---

## Level 19 — Zero Trust Architecture

| Area | Target |
|---|---|
| **Identity-based access** | Every request authenticated and authorized |
| **Microsegmentation** | Network-level isolation between components |
| **Least privilege** | Minimal permissions for all service accounts |
| **Continuous verification** | Re-authentication on every sensitive action |
| **Device trust** | Device posture verification for admin access |
| **Just-in-time access** | Temporary, scoped access for maintenance |

---

## Level 20 — Enterprise Grade

| Area | Target |
|---|---|
| **SOC2 Type II** | Annual audit with clean opinion |
| **ISO 27001** | Certified ISMS |
| **Penetration testing** | Annual third-party pen test |
| **Bug bounty** | Public bug bounty program |
| **Red team exercises** | Quarterly adversarial simulations |
| **Insurance** | Cyber liability insurance coverage |
| **Compliance certifications** | FedRAMP, HIPAA, PCI-DSS as applicable |
| **Enterprise support** | 24/7 support with SLA guarantees |
| **Threat intelligence** | MITRE ATT&CK integration, threat feeds |
| **Security champion program** | Embedded security advocates in every team |

---

## Current Progress

| Milestone | Levels | Status | Date |
|---|---|---|---|
| Open Source Prototype | 0 | ✅ Complete | 2025-01 |
| Supply Chain Foundation | 1 | ✅ Complete | 2026-08-28 |
| Build Provenance | 2 | ✅ Complete | 2026-08-28 |
| Security Hardening | 3 | ✅ Complete | 2026-08-28 |
| Quality Gates | 4 | ✅ Complete | 2026-08-28 |
| Observability | 5 | 🔜 Next | — |
| Enterprise Grade | 20 | ⬜ Target | — |

**Current level: 4 / 20** — 20% towards enterprise readiness

---

*This document is maintained as part of Talus's enterprise maturity program.*
*Updated: 2026-08-28*
