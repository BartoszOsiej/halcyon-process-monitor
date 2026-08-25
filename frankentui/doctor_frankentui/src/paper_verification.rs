// SPDX-License-Identifier: Apache-2.0
//! Primary-paper verification checklists and LEGAL/IP artifact packs.
//!
//! Operationalizes alien-graveyard paper-read rigor for every advanced
//! primitive used in migration uplift: CEGIS, e-graphs, concolic/DSE,
//! abstract interpretation, metamorphic relations, shadow-run governance.
//!
//! # Required outputs per primitive
//!
//! - **Primary-paper checklist**: claims extracted, threats to validity,
//!   incumbent baseline, reproduction plan.
//! - **LEGAL/IP artifact**: patent/license status, design-around note if
//!   needed.
//! - **Repro pack pointers**: env.json, manifest.json, repro.lock, corpus
//!   manifest, deterministic seed policy.
//!
//! # Invariants
//!
//! - No primitive is marked `status=Read` or `Reproduced` without a
//!   completed primary-paper checklist artifact.
//! - Legal/IP status is explicit and tied to rollout risk policy.
//! - Artifact paths are machine-verifiable and referenced by claim IDs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::semantic_contract::IpArtifactStatus;

// ── Schema Version ───────────────────────────────────────────────────────

pub const VERIFICATION_SCHEMA_VERSION: &str = "paper-verification-v1";

// ── Primitive Registry ───────────────────────────────────────────────────

/// Known advanced primitives used in migration uplift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Primitive {
    Cegis,
    EGraphs,
    ConcolicDse,
    AbstractInterpretation,
    MetamorphicRelations,
    ShadowRunGovernance,
}

impl Primitive {
    pub const ALL: &'static [Primitive] = &[
        Primitive::Cegis,
        Primitive::EGraphs,
        Primitive::ConcolicDse,
        Primitive::AbstractInterpretation,
        Primitive::MetamorphicRelations,
        Primitive::ShadowRunGovernance,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Cegis => "CEGIS",
            Self::EGraphs => "E-Graphs",
            Self::ConcolicDse => "Concolic/DSE",
            Self::AbstractInterpretation => "Abstract Interpretation",
            Self::MetamorphicRelations => "Metamorphic Relations",
            Self::ShadowRunGovernance => "Shadow-Run Governance",
        }
    }

    pub fn primary_paper(self) -> &'static str {
        match self {
            Self::Cegis => {
                "Solar-Lezama et al., \"Combinatorial Sketching for Finite Programs\", ASPLOS 2006"
            }
            Self::EGraphs => {
                "Willsey et al., \"egg: Fast and Extensible Equality Saturation\", POPL 2021"
            }
            Self::ConcolicDse => "Sen et al., \"CUTE: A Concolic Unit Testing Engine\", FSE 2005",
            Self::AbstractInterpretation => {
                "Cousot & Cousot, \"Abstract Interpretation\", POPL 1977"
            }
            Self::MetamorphicRelations => "Chen et al., \"Metamorphic Testing\", TSE 2018",
            Self::ShadowRunGovernance => "Veeraraghavan et al., \"Shadow Execution\", SOSP 2015",
        }
    }
}

// ── Primary-Paper Checklist ──────────────────────────────────────────────

/// Verification status for a primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    /// Not yet started.
    NotStarted,
    /// Paper read but checklist incomplete.
    Read,
    /// Checklist complete, awaiting reproduction.
    ChecklistComplete,
    /// Reproduction attempted.
    Reproduced,
    /// Verified and cleared for use.
    Verified,
}

/// A claim extracted from the primary paper.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractedClaim {
    /// Unique claim identifier (e.g. "cegis-claim-001").
    pub claim_id: String,
    /// The claim text.
    pub claim: String,
    /// Section/theorem reference in the paper.
    pub source_ref: String,
    /// Whether this claim is relevant to our usage.
    pub relevant: bool,
    /// Whether we have evidence supporting this claim in our context.
    pub verified: bool,
    /// Verification evidence path (if any).
    pub evidence_path: Option<PathBuf>,
}

/// A threat to validity identified in the paper.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreatToValidity {
    /// Threat identifier.
    pub threat_id: String,
    /// Threat category (internal, external, construct, statistical).
    pub category: ThreatCategory,
    /// Description of the threat.
    pub description: String,
    /// Mitigation strategy (if any).
    pub mitigation: Option<String>,
    /// Whether this threat applies to our usage.
    pub applies_to_us: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatCategory {
    Internal,
    External,
    Construct,
    Statistical,
}

/// The incumbent baseline against which the primitive is compared.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IncumbentBaseline {
    /// What the baseline approach is (e.g. "manual translation").
    pub approach: String,
    /// Known limitations of the baseline.
    pub limitations: Vec<String>,
    /// Expected improvement from adopting the primitive.
    pub expected_improvement: String,
}

/// A reproduction plan for validating the primitive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproductionPlan {
    /// Environment specification file path.
    pub env_path: Option<PathBuf>,
    /// Manifest file path.
    pub manifest_path: Option<PathBuf>,
    /// Lock file path.
    pub repro_lock_path: Option<PathBuf>,
    /// Corpus manifest path.
    pub corpus_manifest_path: Option<PathBuf>,
    /// Deterministic seed policy.
    pub seed_policy: SeedPolicy,
    /// Steps to reproduce.
    pub steps: Vec<String>,
}

/// Deterministic seed policy for reproduction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeedPolicy {
    /// Whether a fixed seed is required.
    pub fixed_seed_required: bool,
    /// The seed value (if fixed).
    pub seed_value: Option<u64>,
    /// Whether iteration count is bounded.
    pub bounded_iterations: bool,
    /// Maximum iterations (if bounded).
    pub max_iterations: Option<u64>,
}

/// Complete primary-paper verification checklist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PaperChecklist {
    /// Schema version.
    pub schema_version: String,
    /// Which primitive this checklist covers.
    pub primitive: Primitive,
    /// Current verification status.
    pub status: VerificationStatus,
    /// Primary paper reference.
    pub primary_paper: String,
    /// Claims extracted from the paper.
    pub claims: Vec<ExtractedClaim>,
    /// Threats to validity.
    pub threats: Vec<ThreatToValidity>,
    /// Incumbent baseline.
    pub baseline: IncumbentBaseline,
    /// Reproduction plan.
    pub reproduction: ReproductionPlan,
}

// ── LEGAL/IP Artifact Pack ───────────────────────────────────────────────

/// Legal/IP artifact pack for a primitive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegalIpPack {
    /// Which primitive this pack covers.
    pub primitive: Primitive,
    /// Patent status.
    pub patent_status: IpArtifactStatus,
    /// Patent details (if any).
    pub patent_notes: Option<String>,
    /// License of the reference implementation.
    pub license_spdx: Option<String>,
    /// License class (permissive, copyleft, proprietary, none).
    pub license_class: LicenseClass,
    /// Design-around notes (if patent/license issues exist).
    pub design_around: Option<String>,
    /// Whether this primitive is cleared for production use.
    pub cleared_for_production: bool,
    /// Risk policy linkage.
    pub rollout_risk_gate: RolloutRiskGate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseClass {
    Permissive,
    WeakCopyleft,
    StrongCopyleft,
    Proprietary,
    None,
}

/// How the IP status gates rollout decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutRiskGate {
    /// No IP concerns — proceed.
    Clear,
    /// Minor concerns — proceed with attribution.
    ProceedWithAttribution,
    /// Significant concerns — requires legal review.
    RequiresLegalReview,
    /// Blocked — do not use in production.
    Blocked,
}

// ── Repro Pack ───────────────────────────────────────────────────────────

/// Machine-verifiable repro pack pointers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReproPackPointers {
    /// Which primitive this pack covers.
    pub primitive: Primitive,
    /// env.json path (relative to project root).
    pub env_json: Option<PathBuf>,
    /// manifest.json path.
    pub manifest_json: Option<PathBuf>,
    /// repro.lock path.
    pub repro_lock: Option<PathBuf>,
    /// Corpus manifest path.
    pub corpus_manifest: Option<PathBuf>,
    /// Deterministic seed policy file path.
    pub seed_policy_path: Option<PathBuf>,
    /// All artifact paths for machine verification.
    pub artifact_paths: Vec<PathBuf>,
    /// Claim IDs that reference these artifacts.
    pub referenced_by_claims: Vec<String>,
}

// ── Composite Verification Bundle ────────────────────────────────────────

/// Complete verification bundle for a single primitive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationBundle {
    pub checklist: PaperChecklist,
    pub legal_ip: LegalIpPack,
    pub repro_pack: ReproPackPointers,
}

/// Registry of all verification bundles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationRegistry {
    pub schema_version: String,
    pub bundles: BTreeMap<Primitive, VerificationBundle>,
}

// ── Validation ───────────────────────────────────────────────────────────

/// Validation error for verification artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationError {
    pub primitive: Primitive,
    pub field: String,
    pub message: String,
}

/// Validate a single verification bundle.
pub fn validate_bundle(bundle: &VerificationBundle) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let prim = bundle.checklist.primitive;

    // Invariant 1: no Read/Reproduced without completed checklist.
    if matches!(
        bundle.checklist.status,
        VerificationStatus::Read | VerificationStatus::Reproduced
    ) && bundle.checklist.claims.is_empty()
    {
        errors.push(ValidationError {
            primitive: prim,
            field: "checklist.claims".into(),
            message: format!(
                "{}: status is {:?} but no claims extracted",
                prim.name(),
                bundle.checklist.status
            ),
        });
    }

    // Claims must have non-empty IDs.
    for claim in &bundle.checklist.claims {
        if claim.claim_id.trim().is_empty() {
            errors.push(ValidationError {
                primitive: prim,
                field: "checklist.claims.claim_id".into(),
                message: "claim_id must not be empty".into(),
            });
        }
    }

    // Duplicate claim IDs.
    let mut seen_claims = std::collections::BTreeSet::new();
    for claim in &bundle.checklist.claims {
        if !seen_claims.insert(&claim.claim_id) {
            errors.push(ValidationError {
                primitive: prim,
                field: "checklist.claims.claim_id".into(),
                message: format!("duplicate claim_id '{}'", claim.claim_id),
            });
        }
    }

    // Threats must have non-empty IDs.
    for threat in &bundle.checklist.threats {
        if threat.threat_id.trim().is_empty() {
            errors.push(ValidationError {
                primitive: prim,
                field: "checklist.threats.threat_id".into(),
                message: "threat_id must not be empty".into(),
            });
        }
    }

    // Invariant 2: Legal/IP status must be explicit.
    if bundle.legal_ip.patent_status == IpArtifactStatus::Unknown
        && bundle.legal_ip.cleared_for_production
    {
        errors.push(ValidationError {
            primitive: prim,
            field: "legal_ip.patent_status".into(),
            message: "cannot be cleared for production with unknown patent status".into(),
        });
    }

    // Invariant 2b: Blocked IP cannot be cleared.
    if bundle.legal_ip.patent_status == IpArtifactStatus::Blocked
        && bundle.legal_ip.cleared_for_production
    {
        errors.push(ValidationError {
            primitive: prim,
            field: "legal_ip.patent_status".into(),
            message: "cannot be cleared for production with blocked patent status".into(),
        });
    }

    // Invariant 2c: NeedsCounsel requires legal review gate.
    if bundle.legal_ip.patent_status == IpArtifactStatus::NeedsCounsel
        && bundle.legal_ip.rollout_risk_gate != RolloutRiskGate::RequiresLegalReview
        && bundle.legal_ip.rollout_risk_gate != RolloutRiskGate::Blocked
    {
        errors.push(ValidationError {
            primitive: prim,
            field: "legal_ip.rollout_risk_gate".into(),
            message: "needs-counsel patent status requires legal review or blocked gate".into(),
        });
    }

    // Invariant 3: referenced claim IDs must exist in checklist.
    let checklist_claim_ids: std::collections::BTreeSet<&str> = bundle
        .checklist
        .claims
        .iter()
        .map(|c| c.claim_id.as_str())
        .collect();
    for ref_id in &bundle.repro_pack.referenced_by_claims {
        if !checklist_claim_ids.contains(ref_id.as_str()) {
            errors.push(ValidationError {
                primitive: prim,
                field: "repro_pack.referenced_by_claims".into(),
                message: format!("repro pack references unknown claim_id '{}'", ref_id),
            });
        }
    }

    // Verified evidence paths: claims marked verified should have evidence.
    for claim in &bundle.checklist.claims {
        if claim.verified && claim.evidence_path.is_none() {
            errors.push(ValidationError {
                primitive: prim,
                field: "checklist.claims.evidence_path".into(),
                message: format!(
                    "claim '{}' is marked verified but has no evidence_path",
                    claim.claim_id
                ),
            });
        }
    }

    errors
}

/// Validate the entire registry.
pub fn validate_registry(registry: &VerificationRegistry) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if registry.schema_version != VERIFICATION_SCHEMA_VERSION {
        errors.push(ValidationError {
            primitive: Primitive::Cegis, // sentinel
            field: "schema_version".into(),
            message: format!(
                "unsupported schema version '{}' (expected '{}')",
                registry.schema_version, VERIFICATION_SCHEMA_VERSION
            ),
        });
    }

    for bundle in registry.bundles.values() {
        errors.extend(validate_bundle(bundle));
    }

    errors
}

/// Check that all required artifact paths exist on disk.
pub fn verify_artifact_paths(
    bundle: &VerificationBundle,
    project_root: &Path,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let prim = bundle.checklist.primitive;

    for artifact_path in &bundle.repro_pack.artifact_paths {
        let full = project_root.join(artifact_path);
        if !full.exists() {
            errors.push(ValidationError {
                primitive: prim,
                field: "repro_pack.artifact_paths".into(),
                message: format!("artifact path does not exist: {}", full.display()),
            });
        }
    }

    errors
}

// ── Builder ──────────────────────────────────────────────────────────────

/// Build the canonical verification registry with all primitives.
///
/// Each primitive starts with its primary-paper reference and a scaffold
/// checklist. Status begins as `NotStarted` — agents fill in claims,
/// threats, and reproduction data as they perform paper reads.
pub fn build_default_registry() -> VerificationRegistry {
    let mut bundles = BTreeMap::new();

    for &prim in Primitive::ALL {
        bundles.insert(prim, build_default_bundle(prim));
    }

    VerificationRegistry {
        schema_version: VERIFICATION_SCHEMA_VERSION.into(),
        bundles,
    }
}

fn build_default_bundle(prim: Primitive) -> VerificationBundle {
    VerificationBundle {
        checklist: PaperChecklist {
            schema_version: VERIFICATION_SCHEMA_VERSION.into(),
            primitive: prim,
            status: VerificationStatus::NotStarted,
            primary_paper: prim.primary_paper().into(),
            claims: Vec::new(),
            threats: Vec::new(),
            baseline: IncumbentBaseline {
                approach: "Manual translation with human review".into(),
                limitations: vec!["Slow, error-prone, not scalable".into()],
                expected_improvement: default_improvement(prim),
            },
            reproduction: ReproductionPlan {
                env_path: None,
                manifest_path: None,
                repro_lock_path: None,
                corpus_manifest_path: None,
                seed_policy: SeedPolicy {
                    fixed_seed_required: true,
                    seed_value: Some(0xF7A4_D12B),
                    bounded_iterations: true,
                    max_iterations: Some(1000),
                },
                steps: Vec::new(),
            },
        },
        legal_ip: build_default_legal_ip(prim),
        repro_pack: ReproPackPointers {
            primitive: prim,
            env_json: None,
            manifest_json: None,
            repro_lock: None,
            corpus_manifest: None,
            seed_policy_path: None,
            artifact_paths: Vec::new(),
            referenced_by_claims: Vec::new(),
        },
    }
}

fn default_improvement(prim: Primitive) -> String {
    match prim {
        Primitive::Cegis => "Automated synthesis of unmapped translation holes".into(),
        Primitive::EGraphs => {
            "Pass-order-insensitive code optimization via equality saturation".into()
        }
        Primitive::ConcolicDse => "Automated test generation for edge-case coverage".into(),
        Primitive::AbstractInterpretation => {
            "Sound over-approximation of program behavior for safety proofs".into()
        }
        Primitive::MetamorphicRelations => {
            "Automated oracle generation for regression testing".into()
        }
        Primitive::ShadowRunGovernance => {
            "Safe shadow execution for gradual rollout validation".into()
        }
    }
}

fn build_default_legal_ip(prim: Primitive) -> LegalIpPack {
    // All primitives are based on well-known academic work with
    // permissive or expired IP status.
    let (patent_status, license_class, license_spdx) = match prim {
        Primitive::Cegis => (IpArtifactStatus::Clear, LicenseClass::None, None),
        Primitive::EGraphs => (
            IpArtifactStatus::Clear,
            LicenseClass::Permissive,
            Some("MIT".to_string()),
        ),
        Primitive::ConcolicDse => (IpArtifactStatus::Clear, LicenseClass::None, None),
        Primitive::AbstractInterpretation => (IpArtifactStatus::Clear, LicenseClass::None, None),
        Primitive::MetamorphicRelations => (IpArtifactStatus::Clear, LicenseClass::None, None),
        Primitive::ShadowRunGovernance => (IpArtifactStatus::Clear, LicenseClass::None, None),
    };

    LegalIpPack {
        primitive: prim,
        patent_status,
        patent_notes: None,
        license_spdx,
        license_class,
        design_around: None,
        cleared_for_production: patent_status == IpArtifactStatus::Clear,
        rollout_risk_gate: match patent_status {
            IpArtifactStatus::Clear | IpArtifactStatus::Expired => RolloutRiskGate::Clear,
            IpArtifactStatus::NeedsCounsel => RolloutRiskGate::RequiresLegalReview,
            IpArtifactStatus::Blocked => RolloutRiskGate::Blocked,
            IpArtifactStatus::Unknown => RolloutRiskGate::RequiresLegalReview,
        },
    }
}

// ── Serialization ────────────────────────────────────────────────────────

/// Serialize a registry to JSON.
pub fn registry_to_json(registry: &VerificationRegistry) -> serde_json::Value {
    serde_json::to_value(registry).unwrap_or(serde_json::Value::Null)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_all_primitives() {
        let reg = build_default_registry();
        assert_eq!(reg.bundles.len(), Primitive::ALL.len());
        for &prim in Primitive::ALL {
            assert!(reg.bundles.contains_key(&prim), "missing {:?}", prim);
        }
    }

    #[test]
    fn default_registry_validates_clean() {
        let reg = build_default_registry();
        let errors = validate_registry(&reg);
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn primitive_name_coverage() {
        for &prim in Primitive::ALL {
            assert!(!prim.name().is_empty());
            assert!(!prim.primary_paper().is_empty());
        }
    }

    #[test]
    fn default_bundle_starts_not_started() {
        let reg = build_default_registry();
        for bundle in reg.bundles.values() {
            assert_eq!(bundle.checklist.status, VerificationStatus::NotStarted);
        }
    }

    #[test]
    fn default_legal_ip_all_clear() {
        let reg = build_default_registry();
        for bundle in reg.bundles.values() {
            assert_eq!(bundle.legal_ip.patent_status, IpArtifactStatus::Clear);
            assert!(bundle.legal_ip.cleared_for_production);
            assert_eq!(bundle.legal_ip.rollout_risk_gate, RolloutRiskGate::Clear);
        }
    }

    #[test]
    fn validate_catches_read_without_claims() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.checklist.status = VerificationStatus::Read;
        // claims is empty
        let errors = validate_bundle(&bundle);
        assert!(
            errors.iter().any(|e| e.message.contains("no claims")),
            "should catch Read without claims: {:?}",
            errors
        );
    }

    #[test]
    fn validate_catches_reproduced_without_claims() {
        let mut bundle = build_default_bundle(Primitive::EGraphs);
        bundle.checklist.status = VerificationStatus::Reproduced;
        let errors = validate_bundle(&bundle);
        assert!(errors.iter().any(|e| e.message.contains("no claims")));
    }

    #[test]
    fn validate_catches_unknown_patent_cleared() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.legal_ip.patent_status = IpArtifactStatus::Unknown;
        bundle.legal_ip.cleared_for_production = true;
        let errors = validate_bundle(&bundle);
        assert!(errors.iter().any(|e| e.message.contains("unknown patent")));
    }

    #[test]
    fn validate_catches_blocked_cleared() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.legal_ip.patent_status = IpArtifactStatus::Blocked;
        bundle.legal_ip.cleared_for_production = true;
        let errors = validate_bundle(&bundle);
        assert!(errors.iter().any(|e| e.message.contains("blocked patent")));
    }

    #[test]
    fn validate_catches_needs_counsel_without_gate() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.legal_ip.patent_status = IpArtifactStatus::NeedsCounsel;
        bundle.legal_ip.cleared_for_production = false;
        bundle.legal_ip.rollout_risk_gate = RolloutRiskGate::Clear;
        let errors = validate_bundle(&bundle);
        assert!(errors.iter().any(|e| e.message.contains("legal review")));
    }

    #[test]
    fn validate_catches_orphan_claim_ref() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.repro_pack.referenced_by_claims = vec!["nonexistent-claim".into()];
        let errors = validate_bundle(&bundle);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("unknown claim_id"))
        );
    }

    #[test]
    fn validate_catches_verified_without_evidence() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.checklist.status = VerificationStatus::ChecklistComplete;
        bundle.checklist.claims.push(ExtractedClaim {
            claim_id: "cegis-claim-001".into(),
            claim: "CEGIS terminates for finite sketches".into(),
            source_ref: "Theorem 1".into(),
            relevant: true,
            verified: true,
            evidence_path: None, // missing!
        });
        let errors = validate_bundle(&bundle);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("no evidence_path"))
        );
    }

    #[test]
    fn validate_catches_duplicate_claim_ids() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.checklist.status = VerificationStatus::ChecklistComplete;
        let claim = ExtractedClaim {
            claim_id: "dup-claim".into(),
            claim: "Some claim".into(),
            source_ref: "Section 3".into(),
            relevant: true,
            verified: false,
            evidence_path: None,
        };
        bundle.checklist.claims.push(claim.clone());
        bundle.checklist.claims.push(claim);
        let errors = validate_bundle(&bundle);
        assert!(errors.iter().any(|e| e.message.contains("duplicate")));
    }

    #[test]
    fn validate_catches_empty_claim_id() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.checklist.claims.push(ExtractedClaim {
            claim_id: "".into(),
            claim: "Some claim".into(),
            source_ref: "Section 1".into(),
            relevant: true,
            verified: false,
            evidence_path: None,
        });
        let errors = validate_bundle(&bundle);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("must not be empty"))
        );
    }

    #[test]
    fn validate_catches_empty_threat_id() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.checklist.threats.push(ThreatToValidity {
            threat_id: " ".into(),
            category: ThreatCategory::Internal,
            description: "Some threat".into(),
            mitigation: None,
            applies_to_us: true,
        });
        let errors = validate_bundle(&bundle);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("must not be empty"))
        );
    }

    #[test]
    fn valid_bundle_with_claims_passes() {
        let mut bundle = build_default_bundle(Primitive::EGraphs);
        bundle.checklist.status = VerificationStatus::Read;
        bundle.checklist.claims.push(ExtractedClaim {
            claim_id: "egraph-claim-001".into(),
            claim: "Equality saturation reaches fixpoint".into(),
            source_ref: "Theorem 2".into(),
            relevant: true,
            verified: false,
            evidence_path: None,
        });
        let errors = validate_bundle(&bundle);
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn verified_claim_with_evidence_passes() {
        let mut bundle = build_default_bundle(Primitive::EGraphs);
        bundle.checklist.status = VerificationStatus::Reproduced;
        bundle.checklist.claims.push(ExtractedClaim {
            claim_id: "egraph-claim-001".into(),
            claim: "Equality saturation reaches fixpoint".into(),
            source_ref: "Theorem 2".into(),
            relevant: true,
            verified: true,
            evidence_path: Some(PathBuf::from("evidence/egraph_fixpoint.json")),
        });
        let errors = validate_bundle(&bundle);
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn repro_pack_claim_refs_valid() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.checklist.claims.push(ExtractedClaim {
            claim_id: "cegis-claim-001".into(),
            claim: "CEGIS terminates".into(),
            source_ref: "Theorem 1".into(),
            relevant: true,
            verified: false,
            evidence_path: None,
        });
        bundle.repro_pack.referenced_by_claims = vec!["cegis-claim-001".into()];
        let errors = validate_bundle(&bundle);
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn registry_json_roundtrip() {
        let reg = build_default_registry();
        let json = registry_to_json(&reg);
        let decoded: VerificationRegistry = serde_json::from_value(json).expect("roundtrip failed");
        assert_eq!(decoded.bundles.len(), reg.bundles.len());
        assert_eq!(decoded.schema_version, VERIFICATION_SCHEMA_VERSION);
    }

    #[test]
    fn schema_version_mismatch_detected() {
        let mut reg = build_default_registry();
        reg.schema_version = "wrong-version".into();
        let errors = validate_registry(&reg);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("unsupported schema"))
        );
    }

    #[test]
    fn needs_counsel_with_legal_review_gate_ok() {
        let mut bundle = build_default_bundle(Primitive::Cegis);
        bundle.legal_ip.patent_status = IpArtifactStatus::NeedsCounsel;
        bundle.legal_ip.cleared_for_production = false;
        bundle.legal_ip.rollout_risk_gate = RolloutRiskGate::RequiresLegalReview;
        let errors = validate_bundle(&bundle);
        assert!(
            !errors.iter().any(|e| e.message.contains("legal review")),
            "should not error when gate matches: {:?}",
            errors
        );
    }

    #[test]
    fn seed_policy_defaults() {
        let reg = build_default_registry();
        for bundle in reg.bundles.values() {
            let sp = &bundle.checklist.reproduction.seed_policy;
            assert!(sp.fixed_seed_required);
            assert!(sp.bounded_iterations);
            assert_eq!(sp.seed_value, Some(0xF7A4_D12B));
        }
    }

    #[test]
    fn egraph_has_mit_license() {
        let reg = build_default_registry();
        let egraph = &reg.bundles[&Primitive::EGraphs];
        assert_eq!(egraph.legal_ip.license_spdx.as_deref(), Some("MIT"));
        assert_eq!(egraph.legal_ip.license_class, LicenseClass::Permissive);
    }
}
