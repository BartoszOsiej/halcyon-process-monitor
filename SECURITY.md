# Security Policy — Talus Process Monitor

> Enterprise-grade vulnerability disclosure and security response policy.

---

## Supported Versions

| Version | Supported | Notes |
|---|---|---|
| 0.5.x (latest) | ✅ | Active development, security patches |
| 0.4.x | ⚠️ | Security fixes only (EOL: 2026-12-31) |
| < 0.4.0 | ❌ | End of life — upgrade recommended |

## Scope

This policy covers the **talus-process-monitor** repository and all official artifacts:

- Rust source code (`process-monitor/`, `process-monitor-ebpf/`)
- C eBPF programs (`c-ebpf/`)
- Go components (`go-agent/`, `go-web/`)
- Tauri desktop app (`talus-tauri/`)
- Docker images published to `ghcr.io`
- Kubernetes manifests (`k8s/`)
- Published binaries and release artifacts

### Out of Scope

- Third-party dependencies (report upstream, then to us)
- Social engineering attacks
- Physical attacks against infrastructure

---

## Reporting a Vulnerability

### ⚠️ Do NOT open a public issue for security vulnerabilities.

Instead, use one of these **private** channels:

| Channel | Best For | Response Time |
|---|---|---|
| **[GitHub Security Advisories](https://github.com/BartoszOsiej/talus-process-monitor/security/advisories/new)** | All vulnerabilities | Primary channel |
| **Email: security@talus.dev** | Sensitive/critical issues | Backup channel |

### What to Include

When reporting, please provide:

1. **Vulnerability type** (e.g., buffer overflow, privilege escalation, RCE)
2. **Affected component** (eBPF, monitor core, TUI, web server, FFI)
3. **Attack vector** (local, network, adjacent)
4. **Reproduction steps** (POC code, logs, crash output)
5. **Impact assessment** (what an attacker could achieve)
6. **Suggested fix** (if you have one)

---

## Response Timeline

| Phase | SLA | Description |
|---|---|---|
| **Acknowledgment** | 48 hours | Confirm receipt, assign tracking ID |
| **Initial Assessment** | 5 business days | Severity classification, triage |
| **Detailed Analysis** | 15 business days | Root cause analysis, fix development |
| **Patch Release** | 30 business days | Security patch shipped |
| **Disclosure** | 45 business days | Coordinated disclosure after patch |

### Severity Classification

| CVSS Score | Severity | Response SLA | Examples |
|---|---|---|---|
| 9.0–10.0 | **Critical** | 48h acknowledgment, 7d fix | Remote code execution, kernel exploit, privilege escalation |
| 7.0–8.9 | **High** | 48h acknowledgment, 14d fix | Memory corruption, authentication bypass |
| 4.0–6.9 | **Medium** | 1 week acknowledgment, 30d fix | Information disclosure, denial of service |
| 0.1–3.9 | **Low** | 2 weeks acknowledgment, 90d fix | Minor information leak, edge-case DoS |

---

## Security Measures

### Supply Chain Security (Level 1–2 ✅)

| Measure | Status | Description |
|---|---|---|
| License compliance | ✅ Active | `cargo-deny` blocks forbidden licenses |
| Advisory audit | ✅ Active | `cargo-deny` blocks known vulnerabilities |
| SBOM generation | ✅ Active | CycloneDX SBOM attached to every release |
| Dependency pinning | ✅ Active | Cargo.lock committed, Dependabot with cooldown |
| Secret scanning | ✅ Active | gitleaks in CI pipeline |
| Binary signing | ✅ Active | Sigstore cosign keyless OIDC signing |
| SLSA provenance | ✅ Active | SLSA Level 2 provenance for all releases |
| Build attestation | ✅ Active | GitHub artifact attestation |

### Runtime Security

| Measure | Description |
|---|---|
| eBPF verifier safety | All kernel programs pass BPF verifier — no unsafe dereferences |
| Privilege model | Requires `CAP_BPF`/`CAP_SYS_ADMIN` — no ambient capability escalation |
| Process isolation | Each tracked PID has isolated sliding window state |
| Input validation | All userspace pointers read via `bpf_probe_read_user` — no TOCTOU |
| Memory safety | Rust ownership model for userspace; `#![no_std]` for kernel |
| Binary hardening | Full LTO, `panic = "abort"`, stripped symbols |

### Build Reproducibility

| Aspect | Detail |
|---|---|
| Pinned toolchain | CI uses specific commit hashes for all actions |
| Deterministic output | `codegen-units = 1`, `strip = "symbols"` |
| Lock file | `Cargo.lock` committed — identical dependency resolution |
| BPF profile | `lto = false`, `opt-level = 2` — avoids LLVM intrinsics that break determinism |

---

## Security Advisories

Published security advisories are available at:
- [GitHub Security Advisories](https://github.com/BartoszOsiej/talus-process-monitor/security/advisories)

### Advisory Format

Each advisory includes:
- CVE identifier (if applicable)
- Affected versions
- Fixed version
- CVSS score and vector
- Remediation instructions

---

## Security Contacts

| Role | Contact |
|---|---|
| Security Lead | Bartosz Osiej |
| Email | security@talus.dev |
| GitHub | [@BartoszOsiej](https://github.com/BartoszOsiej) |
| Advisory Portal | [GitHub Security](https://github.com/BartoszOsiej/talus-process-monitor/security) |

---

## Recognition

We value responsible disclosure. Security researchers who report valid vulnerabilities will be:

1. **Acknowledged** in the security advisory (unless anonymity is requested)
2. **Credited** in the CHANGELOG
3. **Listed** in our Security Hall of Fame (below)

### Hall of Fame

| Researcher | Date | Vulnerability | Severity |
|---|---|---|---|
| — | — | *No reported vulnerabilities yet* | — |

---

## Compliance

This security policy aligns with:

- **NIST SP 800-40** — Guide to Enterprise Patch Management
- **CERT Coordination Center** — Vulnerability Reporting Guidelines
- **ISO 27001:2022** — A.8.8 Technical vulnerability management
- **SOC2 Trust Service Criteria** — CC6.1 Logical access controls

---

*Policy version: 2.0 · Effective: 2026-08-28 · Review: 2027-02-28*
