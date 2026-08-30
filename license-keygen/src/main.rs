//! Talus License Key Generator
//!
//! Generates Ed25519 keypairs for license signing and creates signed license keys.
//!
//! Usage:
//!   talus-keygen init              Generate a new keypair
//!   talus-keygen issue             Issue a new license key
//!   talus-keygen verify <KEY>      Verify a license key
//!   talus-keygen list              List issued licenses
//!   talus-keygen export-public     Export the public key for embedding

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

// ── Key storage ───────────────────────────────────────────────────────────

const KEY_FILE: &str = "signing_key.json";
const LICENSES_FILE: &str = "issued_licenses.json";

fn keys_dir() -> PathBuf {
    PathBuf::from("license-keys")
}

fn signing_key_path() -> PathBuf {
    keys_dir().join(KEY_FILE)
}

fn licenses_path() -> PathBuf {
    keys_dir().join(LICENSES_FILE)
}

// ── Persistent types ──────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct SigningKeyData {
    /// Hex-encoded Ed25519 signing key (private).
    private_key_hex: String,
    /// Hex-encoded Ed25519 verifying key (public).
    public_key_hex: String,
    /// When the key was generated.
    created_at: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct IssuedLicense {
    license_id: String,
    tier: String,
    organization: Option<String>,
    expires_at: Option<String>,
    issued_at: String,
    key_string: String,
    features: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Default)]
struct LicenseRegistry {
    keys: Option<SigningKeyData>,
    licenses: Vec<IssuedLicense>,
}

impl LicenseRegistry {
    fn load() -> Self {
        let path = licenses_path();
        if path.exists() {
            let data = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    fn save(&self) -> Result<()> {
        let dir = keys_dir();
        fs::create_dir_all(&dir)
            .context("failed to create license-keys directory")?;
        let data = serde_json::to_string_pretty(self)
            .context("failed to serialize registry")?;
        fs::write(licenses_path(), data)
            .context("failed to write registry")?;
        Ok(())
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "talus-keygen",
    about = "License key generator for Talus Process Monitor",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new Ed25519 signing keypair
    Init,

    /// Issue a new signed license key
    Issue {
        /// License tier: community or enterprise
        #[arg(long, default_value = "enterprise")]
        tier: String,

        /// Organization name
        #[arg(long)]
        organization: Option<String>,

        /// Expiration date (YYYY-MM-DD). Omit for perpetual license.
        #[arg(long)]
        expires: Option<String>,

        /// License ID (auto-generated if omitted)
        #[arg(long)]
        license_id: Option<String>,

        /// Maximum managed nodes (0 = unlimited)
        #[arg(long, default_value_t = 0)]
        max_nodes: u32,

        /// Explicit feature list (comma-separated). Omit for tier defaults.
        #[arg(long)]
        features: Option<String>,
    },

    /// Verify a license key signature
    Verify {
        /// The license key string to verify
        key: String,
    },

    /// List all issued licenses
    List,

    /// Export the public key for embedding in the binary
    ExportPublic,

    /// Revoke a license by ID
    Revoke {
        /// License ID to revoke
        license_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => cmd_init()?,
        Commands::Issue {
            tier,
            organization,
            expires,
            license_id,
            max_nodes,
            features,
        } => cmd_issue(tier, organization, expires, license_id, max_nodes, features)?,
        Commands::Verify { key } => cmd_verify(key)?,
        Commands::List => cmd_list()?,
        Commands::ExportPublic => cmd_export_public()?,
        Commands::Revoke { license_id } => cmd_revoke(license_id)?,
    }

    Ok(())
}

// ── Commands ──────────────────────────────────────────────────────────────

fn cmd_init() -> Result<()> {
    let dir = keys_dir();
    let key_path = signing_key_path();

    if key_path.exists() {
        println!("⚠ Signing key already exists at: {}", key_path.display());
        println!("  Delete it first if you want to generate a new one.");
        println!("  WARNING: This will invalidate all existing licenses!");
        return Ok(());
    }

    fs::create_dir_all(&dir)?;

    println!("🔑 Generating Ed25519 signing keypair...");

    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();

    let data = SigningKeyData {
        private_key_hex: hex::encode(signing_key.to_bytes()),
        public_key_hex: hex::encode(verifying_key.to_bytes()),
        created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    };

    let json = serde_json::to_string_pretty(&data)?;
    fs::write(&key_path, &json)?;

    println!("✓ Signing key saved to: {}", key_path.display());
    println!();
    println!("Public key (hex):");
    println!("  {}", data.public_key_hex);
    println!();
    println!("⚠ Keep the signing key SECRET!");
    println!("  The public key will be embedded in the Talus binary.");
    println!();
    println!("Next steps:");
    println!("  1. Update PUBLIC_KEY_BYTES in process-monitor/src/license.rs");
    println!("  2. Run: talus-keygen issue --tier enterprise --organization <name>");

    Ok(())
}

fn cmd_issue(
    tier: String,
    organization: Option<String>,
    expires: Option<String>,
    license_id: Option<String>,
    max_nodes: u32,
    features: Option<String>,
) -> Result<()> {
    let key_data = load_signing_key()?;

    // Parse the signing key
    let private_bytes: [u8; 32] = hex::decode(&key_data.private_key_hex)
        .context("invalid private key hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("private key must be 32 bytes"))?;

    let signing_key = SigningKey::from_bytes(&private_bytes);

    // Parse tier
    let tier_lower = tier.to_lowercase();
    if tier_lower != "community" && tier_lower != "enterprise" {
        bail!("invalid tier: must be 'community' or 'enterprise'");
    }

    // Parse features
    let features_list = features.map(|f| {
        f.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<String>>()
    });

    // Parse expiry
    let expires_at = if let Some(ref exp) = expires {
        // Validate format
        chrono::NaiveDate::parse_from_str(exp, "%Y-%m-%d")
            .context("invalid expiry date format: use YYYY-MM-DD")?;
        Some(format!("{exp}T00:00:00Z"))
    } else {
        None
    };

    // Generate license ID
    let id = license_id.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        format!("TALUS-{:04X}-{:04X}-{:04X}", (ts >> 32) & 0xFFFF, (ts >> 16) & 0xFFFF, ts & 0xFFFF)
    });

    // Build payload
    let payload = serde_json::json!({
        "license_id": id,
        "tier": tier_lower,
        "features": features_list,
        "max_nodes": max_nodes,
        "expires_at": expires_at,
        "issued_at": Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "machine_id": null,
        "organization": organization,
    });

    let payload_bytes = serde_json::to_vec(&payload)?;

    // Sign
    let signature = signing_key.sign(&payload_bytes);

    // Encode
    let payload_b64 = BASE64.encode(&payload_bytes);
    let sig_b64 = BASE64.encode(signature.to_bytes());
    let key_string = format!("{payload_b64}.{sig_b64}");

    // Register
    let mut registry = LicenseRegistry::load();
    registry.licenses.push(IssuedLicense {
        license_id: id.clone(),
        tier: tier_lower.clone(),
        organization: organization.clone(),
        expires_at: expires_at.clone(),
        issued_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        key_string: key_string.clone(),
        features: features_list.clone(),
    });
    registry.save()?;

    // Display
    println!();
    println!("═══════════════════════════════════════════════════════════");
    println!("  LICENSE KEY GENERATED");
    println!("═══════════════════════════════════════════════════════════");
    println!();
    println!("  License ID:    {id}");
    println!("  Tier:          {tier}");
    if let Some(ref org) = organization {
        println!("  Organization:  {org}");
    }
    if let Some(ref exp) = expires {
        println!("  Expires:       {exp}");
    } else {
        println!("  Expires:       never (perpetual)");
    }
    println!("  Max nodes:     {}", if max_nodes == 0 { "unlimited".into() } else { max_nodes.to_string() });
    if let Some(ref f) = features_list {
        println!("  Features:      {}", f.join(", "));
    } else {
        println!("  Features:      (tier defaults)");
    }
    println!();
    println!("  ┌─────────────────────────────────────────────────────┐");
    println!("  │  LICENSE KEY                                        │");
    println!("  ├─────────────────────────────────────────────────────┤");

    // Wrap key for display
    let key_display = wrap_text(&key_string, 52);
    for line in &key_display {
        println!("  │  {:<51} │", line);
    }

    println!("  └─────────────────────────────────────────────────────┘");
    println!();
    println!("  Activation command:");
    println!("    talus --license activate <KEY>");
    println!("═══════════════════════════════════════════════════════════");

    Ok(())
}

fn cmd_verify(key_str: String) -> Result<()> {
    let key_data = load_signing_key()?;

    // Parse the public key
    let public_bytes: [u8; 32] = hex::decode(&key_data.public_key_hex)
        .context("invalid public key hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;

    let verifying_key = VerifyingKey::from_bytes(&public_bytes)?;

    // Split key
    let parts: Vec<&str> = key_str.split('.').collect();
    if parts.len() != 2 {
        bail!("invalid key format: expected '<payload>.<signature>'");
    }

    let payload_bytes = BASE64.decode(parts[0])?;
    let sig_bytes = BASE64.decode(parts[1])?;

    let signature = ed25519_dalek::Signature::from_slice(&sig_bytes)?;

    match verifying_key.verify(&payload_bytes, &signature) {
        Ok(()) => {
            let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)?;
            println!("✓ License key is VALID");
            println!("  License ID: {}", payload["license_id"]);
            println!("  Tier:       {}", payload["tier"]);
            if let Some(org) = payload["organization"].as_str() {
                println!("  Organization: {org}");
            }
            Ok(())
        }
        Err(e) => {
            bail!("✗ License key is INVALID: {e}");
        }
    }
}

fn cmd_list() -> Result<()> {
    let registry = LicenseRegistry::load();

    if registry.licenses.is_empty() {
        println!("No licenses issued yet.");
        println!("Run: talus-keygen issue --tier enterprise");
        return Ok(());
    }

    println!();
    println!("═══════════════════════════════════════════════════════════");
    println!("  ISSUED LICENSES ({})", registry.licenses.len());
    println!("═══════════════════════════════════════════════════════════");

    for (i, lic) in registry.licenses.iter().enumerate() {
        println!();
        println!("  #{} │ {}", i + 1, lic.license_id);
        println!("     │ Tier:       {}", lic.tier);
        if let Some(ref org) = lic.organization {
            println!("     │ Org:        {org}");
        }
        println!("     │ Issued:     {}", lic.issued_at);
        if let Some(ref exp) = lic.expires_at {
            println!("     │ Expires:    {exp}");
        } else {
            println!("     │ Expires:    never");
        }
    }

    println!();
    println!("═══════════════════════════════════════════════════════════");

    Ok(())
}

fn cmd_export_public() -> Result<()> {
    let key_data = load_signing_key()?;

    let public_bytes: Vec<u8> = hex::decode(&key_data.public_key_hex)
        .context("invalid public key hex")?;

    println!("// Copy this into process-monitor/src/license.rs");
    println!("// Replace the PUBLIC_KEY_BYTES constant:");
    println!();
    println!("const PUBLIC_KEY_BYTES: [u8; 32] = [");
    for (i, chunk) in public_bytes.chunks(8).enumerate() {
        let hex_vals: Vec<String> = chunk.iter().map(|b| format!("0x{:02x}", b)).collect();
        let suffix = if i < 3 { "," } else { "," };
        println!("    {}, // {}", hex_vals.join(", "), suffix);
    }
    println!("];");

    Ok(())
}

fn cmd_revoke(license_id: String) -> Result<()> {
    let mut registry = LicenseRegistry::load();
    let before = registry.licenses.len();
    registry.licenses.retain(|l| l.license_id != license_id);
    let after = registry.licenses.len();

    if before == after {
        println!("License {license_id} not found.");
    } else {
        registry.save()?;
        println!("✓ License {license_id} revoked.");
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn load_signing_key() -> Result<SigningKeyData> {
    let path = signing_key_path();
    if !path.exists() {
        bail!(
            "no signing key found at: {}\nRun `talus-keygen init` first.",
            path.display()
        );
    }
    let data = fs::read_to_string(&path)?;
    let key: SigningKeyData = serde_json::from_str(&data)?;
    Ok(key)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut remaining = text;
    while remaining.len() > width {
        lines.push(remaining[..width].to_string());
        remaining = &remaining[width..];
    }
    if !remaining.is_empty() {
        lines.push(remaining.to_string());
    }
    lines
}
