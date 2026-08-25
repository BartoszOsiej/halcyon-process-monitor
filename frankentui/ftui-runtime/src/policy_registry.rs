#![forbid(unsafe_code)]

//! Thread-safe registry of named [`PolicyConfig`] instances with lock-free
//! reads and atomic hot-swap via [`arc_swap::ArcSwap`].
//!
//! The registry always contains a `"standard"` policy that matches
//! `PolicyConfig::default()`. Additional named policies can be registered
//! and activated at runtime without dropping frames.
//!
//! # Example
//!
//! ```rust
//! use ftui_runtime::policy_registry::PolicyRegistry;
//! use ftui_runtime::policy_config::PolicyConfig;
//!
//! let registry = PolicyRegistry::new();
//!
//! // Default active policy is "standard"
//! assert_eq!(registry.active_name(), "standard");
//!
//! // Register a custom policy
//! let mut aggressive = PolicyConfig::default();
//! aggressive.conformal.alpha = 0.01;
//! registry.register("aggressive", aggressive);
//!
//! // Hot-swap
//! assert!(registry.set_active("aggressive").is_ok());
//! assert_eq!(registry.active_name(), "aggressive");
//! ```

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use arc_swap::ArcSwap;

use crate::policy_config::PolicyConfig;

/// Default policy name, matching `PolicyConfig::default()`.
pub const STANDARD_POLICY: &str = "standard";

// ---------------------------------------------------------------------------
// Active snapshot
// ---------------------------------------------------------------------------

/// Snapshot of the currently active policy (name + config), stored in
/// the `ArcSwap` for lock-free reads.
#[derive(Debug, Clone)]
struct ActivePolicy {
    name: String,
    config: PolicyConfig,
}

// ---------------------------------------------------------------------------
// Switch event
// ---------------------------------------------------------------------------

/// Record emitted when the active policy changes.
#[derive(Debug, Clone)]
pub struct PolicySwitchEvent {
    /// Name of the previously active policy.
    pub old_name: String,
    /// Name of the newly active policy.
    pub new_name: String,
    /// Monotonic switch counter.
    pub switch_id: u64,
}

impl PolicySwitchEvent {
    /// Serialize as a single-line JSONL string (no trailing newline).
    pub fn to_jsonl(&self) -> String {
        format!(
            r#"{{"schema":"policy-switch-v1","switch_id":{},"old":"{}","new":"{}"}}"#,
            self.switch_id,
            self.old_name.replace('"', "\\\""),
            self.new_name.replace('"', "\\\""),
        )
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from [`PolicyRegistry`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyRegistryError {
    /// The requested policy name is not registered.
    NotFound(String),
    /// Cannot remove or overwrite the built-in standard policy.
    StandardPolicyProtected,
    /// The policy failed validation.
    ValidationFailed(Vec<String>),
}

impl fmt::Display for PolicyRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "policy not found: {name}"),
            Self::StandardPolicyProtected => write!(f, "cannot remove standard policy"),
            Self::ValidationFailed(errors) => {
                write!(f, "policy validation failed: {}", errors.join("; "))
            }
        }
    }
}

impl std::error::Error for PolicyRegistryError {}

// ---------------------------------------------------------------------------
// PolicyRegistry
// ---------------------------------------------------------------------------

/// Thread-safe registry of named [`PolicyConfig`] instances.
///
/// - **Reads are lock-free** via `ArcSwap` (the active policy can be read
///   from any thread without contention).
/// - **Writes** (register/remove/switch) take a brief `RwLock` on the
///   policy map. Since policy changes are rare (operator-initiated), this
///   is not a contention concern.
pub struct PolicyRegistry {
    /// Named policy storage. Write-locked only on register/remove.
    policies: RwLock<HashMap<String, PolicyConfig>>,
    /// The currently active policy (name + config). Lock-free reads.
    active: ArcSwap<ActivePolicy>,
    /// Monotonic switch counter.
    switch_count: std::sync::atomic::AtomicU64,
}

impl PolicyRegistry {
    /// Create a new registry with the `"standard"` policy active.
    pub fn new() -> Self {
        let standard = PolicyConfig::default();
        let mut map = HashMap::new();
        map.insert(STANDARD_POLICY.to_string(), standard.clone());

        Self {
            policies: RwLock::new(map),
            active: ArcSwap::from_pointee(ActivePolicy {
                name: STANDARD_POLICY.to_string(),
                config: standard,
            }),
            switch_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get the currently active policy config (lock-free).
    pub fn active_config(&self) -> PolicyConfig {
        self.active.load().config.clone()
    }

    /// Get the name of the currently active policy (lock-free).
    pub fn active_name(&self) -> String {
        self.active.load().name.clone()
    }

    /// Register a named policy. Validates before accepting.
    ///
    /// Overwrites any existing policy with the same name (except `"standard"`
    /// which is protected).
    pub fn register(&self, name: &str, config: PolicyConfig) -> Result<(), PolicyRegistryError> {
        if name == STANDARD_POLICY {
            return Err(PolicyRegistryError::StandardPolicyProtected);
        }

        let errors = config.validate();
        if !errors.is_empty() {
            return Err(PolicyRegistryError::ValidationFailed(errors));
        }

        let mut map = self.policies.write().unwrap_or_else(|e| e.into_inner());
        map.insert(name.to_string(), config);
        Ok(())
    }

    /// Remove a named policy. Cannot remove `"standard"` or the currently
    /// active policy.
    pub fn remove(&self, name: &str) -> Result<(), PolicyRegistryError> {
        if name == STANDARD_POLICY {
            return Err(PolicyRegistryError::StandardPolicyProtected);
        }

        // Prevent removing the active policy
        if self.active_name() == name {
            return Err(PolicyRegistryError::NotFound(format!(
                "cannot remove active policy: {name}"
            )));
        }

        let mut map = self.policies.write().unwrap_or_else(|e| e.into_inner());
        map.remove(name)
            .map(|_| ())
            .ok_or_else(|| PolicyRegistryError::NotFound(name.to_string()))
    }

    /// Switch the active policy to the named policy.
    ///
    /// Returns a [`PolicySwitchEvent`] recording the transition.
    /// The caller is responsible for emitting it to the evidence ledger
    /// and resetting conformal calibration windows as needed.
    pub fn set_active(&self, name: &str) -> Result<PolicySwitchEvent, PolicyRegistryError> {
        let map = self.policies.read().unwrap_or_else(|e| e.into_inner());
        let config = map
            .get(name)
            .cloned()
            .ok_or_else(|| PolicyRegistryError::NotFound(name.to_string()))?;
        drop(map);

        let old_name = self.active_name();
        let switch_id = self
            .switch_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        self.active.store(Arc::new(ActivePolicy {
            name: name.to_string(),
            config,
        }));

        Ok(PolicySwitchEvent {
            old_name,
            new_name: name.to_string(),
            switch_id,
        })
    }

    /// List all registered policy names.
    pub fn list(&self) -> Vec<String> {
        let map = self.policies.read().unwrap_or_else(|e| e.into_inner());
        let mut names: Vec<String> = map.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get a specific named policy config, if it exists.
    pub fn get(&self, name: &str) -> Option<PolicyConfig> {
        let map = self.policies.read().unwrap_or_else(|e| e.into_inner());
        map.get(name).cloned()
    }

    /// Total number of policy switches performed.
    pub fn switch_count(&self) -> u64 {
        self.switch_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Default for PolicyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for PolicyRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PolicyRegistry")
            .field("active", &self.active_name())
            .field("policies", &self.list())
            .field("switch_count", &self.switch_count())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_standard_policy() {
        let reg = PolicyRegistry::new();
        assert_eq!(reg.active_name(), STANDARD_POLICY);
        assert_eq!(reg.list(), vec![STANDARD_POLICY.to_string()]);
    }

    #[test]
    fn register_and_switch() {
        let reg = PolicyRegistry::new();
        let mut custom = PolicyConfig::default();
        custom.conformal.alpha = 0.01;

        reg.register("custom", custom).unwrap();
        let event = reg.set_active("custom").unwrap();

        assert_eq!(event.old_name, STANDARD_POLICY);
        assert_eq!(event.new_name, "custom");
        assert_eq!(event.switch_id, 0);
        assert_eq!(reg.active_name(), "custom");
        assert!((reg.active_config().conformal.alpha - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn switch_back_to_standard() {
        let reg = PolicyRegistry::new();
        let custom = PolicyConfig::default();
        reg.register("custom", custom).unwrap();
        reg.set_active("custom").unwrap();

        let event = reg.set_active(STANDARD_POLICY).unwrap();
        assert_eq!(event.old_name, "custom");
        assert_eq!(event.new_name, STANDARD_POLICY);
        assert_eq!(event.switch_id, 1);
        assert_eq!(reg.switch_count(), 2);
    }

    #[test]
    fn switch_to_nonexistent_fails() {
        let reg = PolicyRegistry::new();
        let err = reg.set_active("nonexistent").unwrap_err();
        assert!(matches!(err, PolicyRegistryError::NotFound(_)));
    }

    #[test]
    fn cannot_overwrite_standard() {
        let reg = PolicyRegistry::new();
        let err = reg
            .register(STANDARD_POLICY, PolicyConfig::default())
            .unwrap_err();
        assert!(matches!(err, PolicyRegistryError::StandardPolicyProtected));
    }

    #[test]
    fn cannot_remove_standard() {
        let reg = PolicyRegistry::new();
        let err = reg.remove(STANDARD_POLICY).unwrap_err();
        assert!(matches!(err, PolicyRegistryError::StandardPolicyProtected));
    }

    #[test]
    fn cannot_remove_active() {
        let reg = PolicyRegistry::new();
        reg.register("custom", PolicyConfig::default()).unwrap();
        reg.set_active("custom").unwrap();
        let err = reg.remove("custom").unwrap_err();
        assert!(matches!(err, PolicyRegistryError::NotFound(_)));
    }

    #[test]
    fn remove_inactive() {
        let reg = PolicyRegistry::new();
        reg.register("custom", PolicyConfig::default()).unwrap();
        assert_eq!(reg.list().len(), 2);

        reg.remove("custom").unwrap();
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn register_validates() {
        let reg = PolicyRegistry::new();
        let mut bad = PolicyConfig::default();
        bad.conformal.alpha = 0.0; // invalid

        let err = reg.register("bad", bad).unwrap_err();
        assert!(matches!(err, PolicyRegistryError::ValidationFailed(_)));
    }

    #[test]
    fn get_existing() {
        let reg = PolicyRegistry::new();
        let config = reg.get(STANDARD_POLICY);
        assert!(config.is_some());
    }

    #[test]
    fn get_nonexistent() {
        let reg = PolicyRegistry::new();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn switch_event_jsonl() {
        let event = PolicySwitchEvent {
            old_name: "standard".into(),
            new_name: "aggressive".into(),
            switch_id: 42,
        };
        let jsonl = event.to_jsonl();
        assert!(jsonl.contains("policy-switch-v1"));
        assert!(jsonl.contains("\"switch_id\":42"));
        assert!(jsonl.contains("\"old\":\"standard\""));
        assert!(jsonl.contains("\"new\":\"aggressive\""));

        // Verify valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&jsonl).unwrap();
        assert!(parsed.is_object());
    }

    #[test]
    fn debug_format() {
        let reg = PolicyRegistry::new();
        let debug = format!("{reg:?}");
        assert!(debug.contains("PolicyRegistry"));
        assert!(debug.contains("standard"));
    }

    #[test]
    fn concurrent_reads_during_switch() {
        let reg = Arc::new(PolicyRegistry::new());
        let mut custom = PolicyConfig::default();
        custom.conformal.alpha = 0.02;
        reg.register("custom", custom).unwrap();

        std::thread::scope(|s| {
            // Reader threads
            for _ in 0..4 {
                let reg = Arc::clone(&reg);
                s.spawn(move || {
                    for _ in 0..100 {
                        let _name = reg.active_name();
                        let _config = reg.active_config();
                        // Must never panic — lock-free reads
                    }
                });
            }

            // Writer thread
            {
                let reg = Arc::clone(&reg);
                s.spawn(move || {
                    for i in 0..50 {
                        if i % 2 == 0 {
                            let _ = reg.set_active("custom");
                        } else {
                            let _ = reg.set_active(STANDARD_POLICY);
                        }
                    }
                });
            }
        });

        // Final state is deterministic (50 switches, last one is set_active("custom") at i=48 (even))
        // Actually not deterministic since threads race. Just verify no panics occurred.
        assert!(reg.switch_count() > 0);
    }

    #[test]
    fn overwrite_registered_policy() {
        let reg = PolicyRegistry::new();
        let mut v1 = PolicyConfig::default();
        v1.conformal.alpha = 0.02;
        reg.register("custom", v1).unwrap();

        let mut v2 = PolicyConfig::default();
        v2.conformal.alpha = 0.03;
        reg.register("custom", v2).unwrap();

        let config = reg.get("custom").unwrap();
        assert!((config.conformal.alpha - 0.03).abs() < f64::EPSILON);
    }
}
