# doctor_frankentui Verification Report (bd-36je8)

Generated: 2026-02-18

This document is the closeout evidence map for the `bd-36je8` verification program, scoped to
the `doctor_frankentui` crate. It is meant to be read alongside:

- `crates/doctor_frankentui/TEST_MATRIX.md` (the canonical acceptance contract)
- `.github/workflows/ci.yml` (PR gate job: `doctor-frankentui-verification`)
- `.github/workflows/doctor_frankentui_extended.yml` (extended lane)

## Executive Summary

The `doctor_frankentui` verification stack is enforced by CI gates that cover:

- unit + integration tests (real subprocess/network/filesystem; no mocked business behavior)
- a no-fake realism policy gate (with explicit allow markers for unavoidable synthetic helpers)
- e2e workflows producing validated JSONL telemetry, per-step logs, and artifact manifests
- determinism soak runs with non-volatile divergence detection and actionable first-diff pointers
- shadow comparisons that pair default and conservative workflows and compare artifact/replay quality
- a coverage gate with explicit per-module floors and deterministic diagnostics

All CI lanes publish a machine-readable artifact index (`artifact_map.txt` and `artifact_map.json`)
so failures are triageable without reruns.

## Evidence Index (CI Artifact Map)

Both the PR gate and extended lane publish:

- `artifact_map.txt` (key=value pointers)
- `artifact_map.json` (machine-readable index with exists + size metadata)

Canonical artifact roots:

- PR gate: `/tmp/doctor_frankentui_ci/`
- Extended lane: `/tmp/doctor_frankentui_ci_extended/`

High-signal artifact_map keys (non-exhaustive):

- `unit_integration_log`
- `no_fake_gate_log`
- `happy_summary`, `happy_events_jsonl`, `happy_events_validation_report`, `happy_artifact_manifest`
- `failure_summary`, `failure_events_jsonl`, `failure_events_validation_report`, `failure_case_results`
- `replay_triage_report_json`
- `determinism_run_index`, `determinism_report_json`, `determinism_report_txt`
- `shadow_report_json`, `shadow_summary_json`, `shadow_summary_txt`, `shadow_artifact_manifest`
- `coverage_report_json`, `coverage_report_txt`, `coverage_thresholds_toml`, `telemetry_schema_json`

## Unit/Integration Coverage Outcomes

Coverage is enforced by an explicit, reproducible policy:

- Policy definition: `crates/doctor_frankentui/coverage/thresholds.toml`
  - includes total floors and per-module floors (each `crates/doctor_frankentui/src/*.rs`)
- Coverage gate runner: `scripts/doctor_frankentui_coverage.sh`
  - produces `coverage_gate_report.json` and `coverage_gate_report.txt`
  - uses `cargo llvm-cov` (branch+line+function coverage)

Unit + integration tests are executed via CI and stored in:

- `unit_integration_log` (from `cargo test -p doctor_frankentui --all-targets -- --nocapture`)

## Realism / No-Fake Compliance Outcomes

The no-fake policy is enforced by an automated gate:

- Gate script: `scripts/doctor_frankentui_no_fake_gate.py`
- CI log artifact: `no_fake_gate_log`

Policy shape:

- Disallows unannotated fake/shim patterns in Rust tests and e2e shell scripts.
- Allows narrowly-scoped synthetic helpers only when annotated with:
  - `doctor_frankentui:no-fake-allow` (with justification near the use site)

## E2E Observability Outcomes

Canonical E2E workflows:

- Happy: `scripts/doctor_frankentui_happy_e2e.sh`
- Failure matrix: `scripts/doctor_frankentui_failure_e2e.sh`

Each workflow emits:

- per-step stdout/stderr logs (see `logs/*.log` under the artifact root)
- `meta/events.jsonl` validated against the schema:
  - schema: `crates/doctor_frankentui/coverage/e2e_jsonl_schema.json`
  - report: `meta/events_validation_report.json`
- `meta/summary.json` and `meta/summary.txt`
- `meta/artifact_manifest.json` (hashes/sizes for primary artifacts)

Early-failure continuity:

- Both e2e scripts finalize telemetry via `EXIT` traps so `events.jsonl`, validation reports,
  summaries, and manifests are emitted even when an early step fails.
- VHS hang detection is fail-fast with a bounded smoke timeout; on timeout the scripts still
  finalize telemetry for postmortem.

Replay/triage helper (failure workflow):

- Script: `scripts/doctor_frankentui_replay_triage.py`
- Output: `replay_triage_report_json`

Shadow comparison suite:

- Script: `scripts/doctor_frankentui_shadow_compare.sh`
- Outputs:
  - `shadow_report_json`
  - `shadow_summary_json`
  - `shadow_summary_txt`
  - `shadow_artifact_manifest`
  - `shadow_replay_sh`

Contract:

- PR gate compares the happy-path workflow in default vs conservative mode.
- Extended lane compares both the happy-path and failure-path workflows.
- The report must surface status drift, missing artifacts, replay-command inventory drift,
  fallback-coverage regressions, and average/tail turnaround metrics.

## Determinism + Flake Diagnostics Outcomes

Determinism soak runner:

- Script: `scripts/doctor_frankentui_determinism_soak.sh`
- Evidence:
  - `determinism_run_index` (TSV index of runs and exit codes)
  - `determinism_report_json` and `determinism_report_txt`

Contract:

- Detects non-volatile divergence across repeated runs.
- Normalizes run-root paths and filters explicitly-declared volatile artifacts.

## CI Gate Outcomes and Lanes

PR gate lane:

- Workflow: `.github/workflows/ci.yml`
- Job: `doctor-frankentui-verification`
- Purpose: enforce regressions deterministically with fast enough runtime for PRs.
- VHS stack is installed/pinned in the job and VHS smoke timeout is bounded via
  `DOCTOR_FRANKENTUI_VHS_SMOKE_TIMEOUT_S`.

Extended lane:

- Workflow: `.github/workflows/doctor_frankentui_extended.yml`
- Purpose: deeper intermittency diagnostics without blocking the PR lane.
- Adds:
  - determinism soak at higher iteration count
  - an additional conservative-mode happy-path run
  - full happy+failure shadow comparison
  - longer artifact retention

## Residual Risk Register

1. External tool variance (VHS/ttyd/ffmpeg/browser)
   - Risk: environment-specific behavior (missing tools, version drift, browser/codec quirks).
   - Mitigation: CI installs/pins VHS stack; e2e emits tool versions and bounded smoke checks.

2. Determinism scope and volatility allowlist completeness
   - Risk: a newly-added volatile artifact could cause false positives/negatives in soak.
   - Mitigation: soak normalizes paths and uses an explicit suffix allowlist; changes should be
     reviewed whenever e2e artifact set changes.

3. Coverage floors are policy-driven, not “100% or bust”
   - Risk: uncovered branches may remain if floors are set below 100%.
   - Mitigation: per-module floors prevent “hide regressions behind averages”; raise floors as
     gaps close.

## Matrix-to-Evidence Mapping Rule (How To Read TEST_MATRIX)

The matrix in `crates/doctor_frankentui/TEST_MATRIX.md` defines per-behavior expectations.
Evidence is linked by rule:

- Rows with `level=unit` or `level=integration`:
  - primary evidence: `unit_integration_log`
  - coverage enforcement: `coverage_report_json` and `coverage_thresholds_toml`
- Rows with `level=e2e`:
  - primary evidence: `happy_*` and `failure_*` artifacts (events JSONL + summaries + manifests)
  - determinism evidence: `determinism_*` artifacts
  - replay/triage evidence: `replay_triage_report_json`
  - shadow evidence: `shadow_*` artifacts

If a row is changed (new behavior, new artifacts, new tools), update:

- the matrix row itself
- the CI artifact map keys (so evidence remains discoverable)
- determinism volatility allowlist (when new volatile artifacts are introduced)
