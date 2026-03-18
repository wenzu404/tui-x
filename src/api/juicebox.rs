//! Juicebox PIN-based key recovery protocol for X/Twitter E2EE DMs.
//!
//! Implements the 3-phase distributed recovery protocol:
//! Phase 1: Get version (registration salt) from realms
//! Phase 2: OPRF exchange (blinded PIN verification)
//! Phase 3: Recover encrypted secret shares + reconstruct key

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Configuration for a single Juicebox realm.
#[derive(Debug, Clone)]
pub struct RealmConfig {
    pub id: [u8; 16],
    pub address: String,
    pub public_key: Option<[u8; 32]>, // X25519 public key for hardware realms
    pub token: String,                // JWT auth token
}

/// Token map from the GetPublicKeys response.
#[derive(Debug, Clone)]
pub struct JuiceboxConfig {
    pub realms: Vec<RealmConfig>,
    pub register_threshold: u8,
    pub recover_threshold: u8,
    pub pin_hashing_mode: String,
}

impl JuiceboxConfig {
    /// Parse from the GetPublicKeys GraphQL response.
    pub fn from_public_keys_response(data: &serde_json::Value, user_id: &str) -> Option<Self> {
        let users = data
            .get("data")
            .and_then(|d| d.get("user_results_by_rest_ids"))
            .and_then(|u| u.as_array())?;

        let user = users.iter().find(|u| {
            u.get("rest_id").and_then(|r| r.as_str()) == Some(user_id)
        })?;

        let pk_result = user
            .get("result")
            .and_then(|r| r.get("get_public_keys"))?;

        let token_map_entry = pk_result
            .get("public_keys_with_token_map")
            .and_then(|p| p.as_array())
            .and_then(|arr| arr.first())?;

        let tm = token_map_entry.get("token_map")?;
        let tm_json: serde_json::Value = tm
            .get("key_store_token_map_json")
            .and_then(|j| j.as_str())
            .and_then(|s| serde_json::from_str(s).ok())?;

        let pin_hashing_mode = tm_json
            .get("pin_hashing_mode")
            .and_then(|p| p.as_str())
            .unwrap_or("Standard2019")
            .to_string();

        let recover_threshold = tm
            .get("recover_threshold")
            .and_then(|r| r.as_u64())
            .unwrap_or(2) as u8;

        let register_threshold = tm
            .get("register_threshold")
            .and_then(|r| r.as_u64())
            .unwrap_or(3) as u8;

        let token_entries = tm.get("token_map").and_then(|t| t.as_array())?;
        let mut realms = Vec::new();

        for entry in token_entries {
            let realm_id_hex = entry.get("key").and_then(|k| k.as_str())?;
            let value = entry.get("value")?;
            let address = value.get("address").and_then(|a| a.as_str())?;
            let token = value.get("token").and_then(|t| t.as_str())?;
            let public_key_hex = value.get("public_key").and_then(|p| p.as_str());

            let mut id = [0u8; 16];
            hex::decode_to_slice(realm_id_hex, &mut id)?;

            let public_key = public_key_hex.and_then(|h| {
                let mut key = [0u8; 32];
                hex::decode_to_slice(h, &mut key)?;
                Some(key)
            });

            realms.push(RealmConfig {
                id,
                address: address.to_string(),
                public_key,
                token: token.to_string(),
            });
        }

        Some(JuiceboxConfig {
            realms,
            register_threshold,
            recover_threshold,
            pin_hashing_mode,
        })
    }
}

/// Stretch the PIN using Argon2id (Standard2019 mode).
/// Returns (access_key[32], encryption_key_seed[32]).
pub fn stretch_pin(pin: &str, version: &[u8; 16], user_info: &[u8]) -> Result<([u8; 32], [u8; 32])> {
    use argon2::Argon2;

    // Build salt: be4(len(version)) || version || be4(len(userInfo)) || userInfo
    let mut salt = Vec::new();
    salt.extend_from_slice(&(version.len() as u32).to_be_bytes());
    salt.extend_from_slice(version);
    salt.extend_from_slice(&(user_info.len() as u32).to_be_bytes());
    salt.extend_from_slice(user_info);

    // Argon2id with Standard2019 params: m=16384, t=32, p=1, output=64
    let params = argon2::Params::new(16384, 32, 1, Some(64))
        .map_err(|e| anyhow::anyhow!("Invalid Argon2 params: {e}"))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut output = [0u8; 64];
    argon2
        .hash_password_into(pin.as_bytes(), &salt, &mut output)
        .map_err(|e| anyhow::anyhow!("Argon2 hash failed: {e}"))?;

    let mut access_key = [0u8; 32];
    let mut encryption_key_seed = [0u8; 32];
    access_key.copy_from_slice(&output[..32]);
    encryption_key_seed.copy_from_slice(&output[32..]);

    // Zeroize stretched pin
    output.fill(0);

    Ok((access_key, encryption_key_seed))
}

// ── Hex helper ──────────────────────────────────────────────────────

mod hex {
    pub fn decode_to_slice(hex: &str, out: &mut [u8]) -> Option<()> {
        if hex.len() != out.len() * 2 {
            return None;
        }
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let hi = from_hex_digit(chunk[0])?;
            let lo = from_hex_digit(chunk[1])?;
            out[i] = (hi << 4) | lo;
        }
        Some(())
    }

    fn from_hex_digit(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
}

// ── Phase 1-3 CBOR request/response types ───────────────────────────

// These match the Juicebox protocol's CBOR-encoded types.
// For software realms (no Noise encryption), we send these directly.
// For hardware realms, they're wrapped in a Noise session.

/// Phase 1: Request registration version.
#[derive(Serialize)]
struct Recover1Request {
    // empty - just the enum tag
}

/// Phase 2: OPRF blinded input.
#[derive(Serialize)]
struct Recover2Request {
    version: Vec<u8>,
    oprf_blinded_input: Vec<u8>,
}

/// Phase 3: Prove PIN knowledge.
#[derive(Serialize)]
struct Recover3Request {
    version: Vec<u8>,
    unlock_key_tag: Vec<u8>,
}

/// The full recovery result.
pub struct RecoveredSecret {
    pub secret: Vec<u8>,
}

// TODO: Full implementation of the 3-phase protocol requires:
// 1. CBOR serialization of SecretsRequest enum variants
// 2. Noise_NK_25519_ChaChaPoly_BLAKE2s handshake for hardware realms
// 3. OPRF over Ristretto255 (blinding, unblinding, DLEQ verification)
// 4. Shamir secret sharing reconstruction over Ristretto255 scalars
// 5. BLAKE2s-MAC for unlock key tags and commitments
// 6. ChaCha20Poly1305 for final secret decryption
//
// This is a significant crypto implementation. For now, we provide:
// - PIN stretching (Argon2id)
// - Config parsing from GetPublicKeys response
// - The framework for the 3-phase protocol
//
// The actual realm communication will be implemented incrementally.

/// Attempt to recover the user's private key using their PIN.
/// This is the main entry point for E2EE DM decryption.
pub async fn recover_private_key(
    http: &reqwest::Client,
    config: &JuiceboxConfig,
    pin: &str,
    user_id: &str,
) -> Result<Vec<u8>> {
    // Phase 1: Get version from realms
    let version = phase1_get_version(http, config).await?;
    tracing::info!("Phase 1 complete: got version");

    // Stretch PIN with Argon2id
    let (access_key, encryption_key_seed) = stretch_pin(pin, &version, user_id.as_bytes())?;
    tracing::info!("PIN stretched");

    // Phase 2: OPRF exchange
    let (unlock_key, _oprf_output) = phase2_oprf(http, config, &version, &access_key).await?;
    tracing::info!("Phase 2 complete: OPRF done");

    // Phase 3: Recover shares + reconstruct
    let secret = phase3_recover(http, config, &version, &unlock_key, &encryption_key_seed).await?;
    tracing::info!("Phase 3 complete: secret recovered ({} bytes)", secret.len());

    Ok(secret)
}

async fn phase1_get_version(
    http: &reqwest::Client,
    config: &JuiceboxConfig,
) -> Result<[u8; 16]> {
    // Send Recover1 to each software realm (no Noise needed)
    for realm in &config.realms {
        if realm.public_key.is_some() {
            // Skip hardware realms for now (need Noise handshake)
            continue;
        }

        // CBOR-encode the Recover1 request
        // SecretsRequest enum: variant index 4 = Recover1
        let request_body = build_recover1_cbor();

        let resp = http
            .post(format!("{}req", realm.address))
            .header("Authorization", format!("Bearer {}", realm.token))
            .body(request_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!("Realm {} phase 1 failed: {status} {body}", realm.address);
            continue;
        }

        let resp_bytes = resp.bytes().await?;
        if let Some(version) = parse_recover1_response(&resp_bytes) {
            return Ok(version);
        }
    }

    anyhow::bail!("Phase 1 failed: no realm returned a valid version")
}

async fn phase2_oprf(
    _http: &reqwest::Client,
    _config: &JuiceboxConfig,
    _version: &[u8; 16],
    _access_key: &[u8; 32],
) -> Result<([u8; 32], [u8; 64])> {
    // TODO: Implement OPRF protocol
    // 1. Blind the access_key using Ristretto255
    // 2. Send blinded value to >= 2 realms
    // 3. Verify DLEQ proofs
    // 4. Reconstruct OPRF output via Lagrange interpolation
    // 5. Derive unlock_key
    anyhow::bail!("OPRF phase not yet implemented - requires Ristretto255 OPRF")
}

async fn phase3_recover(
    _http: &reqwest::Client,
    _config: &JuiceboxConfig,
    _version: &[u8; 16],
    _unlock_key: &[u8; 32],
    _encryption_key_seed: &[u8; 32],
) -> Result<Vec<u8>> {
    // TODO: Implement share recovery
    // 1. Compute unlock_key_tag for each realm
    // 2. Send Recover3 to >= 2 realms
    // 3. Reconstruct encryption_key_scalar via Lagrange
    // 4. Derive encryption_key via BLAKE2s-MAC
    // 5. Decrypt encrypted_secret via ChaCha20Poly1305
    anyhow::bail!("Phase 3 not yet implemented - requires Shamir reconstruction")
}

// ── CBOR helpers ────────────────────────────────────────────────────

fn build_recover1_cbor() -> Vec<u8> {
    // SecretsRequest is a CBOR enum. In ciborium:
    // Recover1 = variant index 4, no fields
    // Encoded as: CBOR map with "Recover1" key -> null
    let mut buf = Vec::new();
    ciborium::into_writer(&ciborium::Value::Map(vec![
        (
            ciborium::Value::Text("Recover1".to_string()),
            ciborium::Value::Null,
        ),
    ]), &mut buf)
    .unwrap();
    buf
}

fn parse_recover1_response(data: &[u8]) -> Option<[u8; 16]> {
    // Response is CBOR: {"Ok": {"version": <16 bytes>}}
    let value: ciborium::Value = ciborium::from_reader(data).ok()?;
    let map = match value {
        ciborium::Value::Map(m) => m,
        _ => return None,
    };

    for (key, val) in &map {
        if let ciborium::Value::Text(k) = key {
            if k == "Ok" {
                if let ciborium::Value::Map(inner) = val {
                    for (ik, iv) in inner {
                        if let ciborium::Value::Text(ik_str) = ik {
                            if ik_str == "version" {
                                if let ciborium::Value::Bytes(bytes) = iv {
                                    if bytes.len() == 16 {
                                        let mut version = [0u8; 16];
                                        version.copy_from_slice(bytes);
                                        return Some(version);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}
