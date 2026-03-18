//! Juicebox PIN-based key recovery protocol for X/Twitter E2EE DMs.
//!
//! Implements the 3-phase distributed recovery protocol:
//! Phase 1: Get version (registration salt) from realms
//! Phase 2: OPRF exchange (blinded PIN verification)
//! Phase 3: Recover encrypted secret shares + reconstruct key

use anyhow::{bail, Context, Result};
use blake2::digest::typenum::{U16, U32};
use blake2::digest::{KeyInit, Mac};
use blake2::Blake2sMac;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::ChaCha20Poly1305;
use ciborium::Value as CborValue;
use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::Identity;
use sha2::{Digest, Sha512};

// ── Type aliases ────────────────────────────────────────────────────

type Blake2sMac128 = Blake2sMac<U16>;
type Blake2sMac256 = Blake2sMac<U32>;

// ── Public types ────────────────────────────────────────────────────

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

        let user = users
            .iter()
            .find(|u| u.get("rest_id").and_then(|r| r.as_str()) == Some(user_id))?;

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

    pub fn encode(data: &[u8]) -> String {
        let mut s = String::with_capacity(data.len() * 2);
        for b in data {
            s.push_str(&format!("{b:02x}"));
        }
        s
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

// ── PIN stretching ──────────────────────────────────────────────────

/// Stretch the PIN using Argon2id (Standard2019 mode).
/// Returns (access_key[32], encryption_key_seed[32]).
pub fn stretch_pin(
    pin: &str,
    version: &[u8; 16],
    user_info: &[u8],
) -> Result<([u8; 32], [u8; 32])> {
    use argon2::Argon2;

    // Build salt: be4(len(version)) || version || be4(len(userInfo)) || userInfo
    let mut salt = Vec::new();
    salt.extend_from_slice(&to_be4(version.len()));
    salt.extend_from_slice(version);
    salt.extend_from_slice(&to_be4(user_info.len()));
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

    output.fill(0);

    Ok((access_key, encryption_key_seed))
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Encode a length as big-endian 4-byte integer.
fn to_be4(len: usize) -> [u8; 4] {
    (len as u32).to_be_bytes()
}

/// Build CBOR bytes from a Value.
fn cbor_encode(val: &CborValue) -> Vec<u8> {
    let mut buf = Vec::new();
    ciborium::into_writer(val, &mut buf).expect("CBOR encode failed");
    buf
}

/// Decode CBOR bytes into a Value.
fn cbor_decode(data: &[u8]) -> Result<CborValue> {
    ciborium::from_reader(data).context("CBOR decode failed")
}

/// Extract a byte-array field from a CBOR map.
fn cbor_get_bytes<'a>(map: &'a [(CborValue, CborValue)], key: &str) -> Option<&'a Vec<u8>> {
    for (k, v) in map {
        if let CborValue::Text(k_str) = k {
            if k_str == key {
                if let CborValue::Bytes(b) = v {
                    return Some(b);
                }
            }
        }
    }
    None
}

/// Extract a u64 field from a CBOR map.
fn cbor_get_u64(map: &[(CborValue, CborValue)], key: &str) -> Option<u64> {
    for (k, v) in map {
        if let CborValue::Text(k_str) = k {
            if k_str == key {
                if let CborValue::Integer(n) = v {
                    return Some((*n).try_into().ok()?);
                }
            }
        }
    }
    None
}

/// Extract a nested CBOR map field.
fn cbor_get_map<'a>(
    map: &'a [(CborValue, CborValue)],
    key: &str,
) -> Option<&'a Vec<(CborValue, CborValue)>> {
    for (k, v) in map {
        if let CborValue::Text(k_str) = k {
            if k_str == key {
                if let CborValue::Map(m) = v {
                    return Some(m);
                }
            }
        }
    }
    None
}

/// Unwrap CBOR `{"Ok": inner}` envelope, returning the inner map entries.
fn cbor_unwrap_ok(val: &CborValue) -> Result<&Vec<(CborValue, CborValue)>> {
    let map = match val {
        CborValue::Map(m) => m,
        _ => bail!("Expected CBOR map at top level"),
    };
    for (k, v) in map {
        if let CborValue::Text(k_str) = k {
            if k_str == "Ok" {
                if let CborValue::Map(inner) = v {
                    return Ok(inner);
                }
            }
            if k_str == "Err" {
                bail!("Realm returned error: {v:?}");
            }
        }
    }
    bail!("Expected Ok or Err key in CBOR response")
}

// ── Noise NK transport for hardware realms ──────────────────────────

/// For hardware realms: wrap the CBOR secrets request in a Noise NK handshake
/// and build the outer ClientRequest CBOR envelope.
fn build_hardware_request(
    realm: &RealmConfig,
    secrets_request_cbor: &[u8],
) -> Result<(Vec<u8>, snow::HandshakeState)> {
    let pub_key = realm
        .public_key
        .context("build_hardware_request called on software realm")?;

    // Build Noise NK initiator
    let params: snow::params::NoiseParams = "Noise_NK_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .map_err(|e| anyhow::anyhow!("Noise params parse: {e}"))?;

    let mut noise = snow::Builder::new(params)
        .remote_public_key(&pub_key)
        .build_initiator()
        .map_err(|e| anyhow::anyhow!("Noise build_initiator: {e}"))?;

    // Write the first handshake message (-> e, es) with the secrets request as payload.
    // For NK: message 1 contains [ephemeral_pub(32) || encrypted_payload(len + 16 AEAD tag)]
    let mut msg_buf = vec![0u8; 65535];
    let msg_len = noise
        .write_message(secrets_request_cbor, &mut msg_buf)
        .map_err(|e| anyhow::anyhow!("Noise write_message: {e}"))?;
    let noise_msg = &msg_buf[..msg_len];

    // Split: first 32 bytes = client ephemeral public key, rest = encrypted payload
    let client_ephemeral_public = noise_msg[..32].to_vec();
    let payload_ciphertext = noise_msg[32..].to_vec();

    // Random session ID
    let session_id: u32 = rand::random();

    // Build the ClientRequest CBOR envelope
    let client_request = CborValue::Map(vec![
        (
            CborValue::Text("realm_id".into()),
            CborValue::Bytes(realm.id.to_vec()),
        ),
        (
            CborValue::Text("auth_token".into()),
            CborValue::Text(realm.token.clone()),
        ),
        (
            CborValue::Text("session_id".into()),
            CborValue::Integer(session_id.into()),
        ),
        (
            CborValue::Text("kind".into()),
            CborValue::Text("SecretsRequest".into()),
        ),
        (
            CborValue::Text("noise_request".into()),
            CborValue::Map(vec![(
                CborValue::Text("Handshake".into()),
                CborValue::Map(vec![
                    (
                        CborValue::Text("client_ephemeral_public".into()),
                        CborValue::Bytes(client_ephemeral_public),
                    ),
                    (
                        CborValue::Text("payload_ciphertext".into()),
                        CborValue::Bytes(payload_ciphertext),
                    ),
                ]),
            )]),
        ),
    ]);

    Ok((cbor_encode(&client_request), noise))
}

/// Decode a hardware realm response: extract the Noise response, decrypt it,
/// and return the inner CBOR SecretsResponse.
fn decode_hardware_response(
    resp_bytes: &[u8],
    mut noise: snow::HandshakeState,
) -> Result<CborValue> {
    let resp_val = cbor_decode(resp_bytes)?;
    let resp_map = match &resp_val {
        CborValue::Map(m) => m,
        _ => bail!("Expected CBOR map in hardware response"),
    };

    // Extract the noise_response field
    let noise_resp = cbor_get_map(resp_map, "noise_response")
        .context("Missing noise_response in hardware response")?;

    // The noise response may be a "Handshake" or "Transport" variant
    // For NK pattern, server sends message 2 (-> e, ee) with encrypted payload
    let handshake_map = cbor_get_map(noise_resp, "Handshake");
    let transport_map = cbor_get_map(noise_resp, "Transport");

    if let Some(hs_map) = handshake_map {
        // Handshake response: server_ephemeral_public + payload_ciphertext
        // We need to read the full Noise message (ephemeral + ciphertext)
        let server_ephemeral =
            cbor_get_bytes(hs_map, "server_ephemeral_public").context("Missing server ephemeral")?;
        let ciphertext =
            cbor_get_bytes(hs_map, "payload_ciphertext").context("Missing payload_ciphertext")?;

        // Reconstruct the full Noise message for read_message
        let mut full_msg = Vec::with_capacity(server_ephemeral.len() + ciphertext.len());
        full_msg.extend_from_slice(server_ephemeral);
        full_msg.extend_from_slice(ciphertext);

        let mut plaintext_buf = vec![0u8; 65535];
        let plaintext_len = noise
            .read_message(&full_msg, &mut plaintext_buf)
            .map_err(|e| anyhow::anyhow!("Noise read_message: {e}"))?;

        cbor_decode(&plaintext_buf[..plaintext_len])
    } else if let Some(tr_map) = transport_map {
        // Transport mode response (already past handshake)
        let ciphertext =
            cbor_get_bytes(tr_map, "ciphertext").context("Missing transport ciphertext")?;

        let mut noise = noise
            .into_transport_mode()
            .map_err(|e| anyhow::anyhow!("Noise into_transport_mode: {e}"))?;

        let mut plaintext_buf = vec![0u8; 65535];
        let plaintext_len = noise
            .read_message(ciphertext, &mut plaintext_buf)
            .map_err(|e| anyhow::anyhow!("Noise transport read: {e}"))?;

        cbor_decode(&plaintext_buf[..plaintext_len])
    } else {
        bail!("Unknown noise_response variant: {noise_resp:?}")
    }
}

// ── Realm communication ─────────────────────────────────────────────

/// Send a secrets request to a realm and get the decoded CBOR response.
/// Handles both software realms (direct CBOR) and hardware realms (Noise NK).
async fn send_realm_request(
    http: &reqwest::Client,
    realm: &RealmConfig,
    secrets_request: &CborValue,
) -> Result<CborValue> {
    let secrets_cbor = cbor_encode(secrets_request);
    let url = format!("{}req", realm.address);

    // Common headers required by all realms
    let version_header = "X-Juicebox-Version";
    let sdk_version = "0.3.6";

    if realm.public_key.is_some() {
        // Hardware realm: wrap in Noise NK handshake
        let (body, noise) = build_hardware_request(realm, &secrets_cbor)?;

        let resp = http
            .post(&url)
            .header(version_header, sdk_version)
            .header("User-Agent", format!("JuiceboxSdk-Rust/{sdk_version}"))
            .header("Content-Type", "application/cbor")
            .body(body)
            .send()
            .await
            .context("Hardware realm request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Hardware realm {} error: {status} {body}", realm.address);
        }

        let resp_bytes = resp.bytes().await?;
        decode_hardware_response(&resp_bytes, noise)
    } else {
        // Software realm: direct CBOR with Bearer auth
        let resp = http
            .post(&url)
            .header("Authorization", format!("Bearer {}", realm.token))
            .header(version_header, sdk_version)
            .header("User-Agent", format!("JuiceboxSdk-Rust/{sdk_version}"))
            .header("Content-Type", "application/cbor")
            .header("Origin", "https://x.com")
            .header("Referer", "https://x.com/")
            .body(secrets_cbor)
            .send()
            .await
            .context("Software realm request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Software realm {} error: {status} {body}", realm.address);
        }

        let resp_bytes = resp.bytes().await?;
        cbor_decode(&resp_bytes)
    }
}

// ── Phase 1: Get Version ────────────────────────────────────────────

fn build_recover1_cbor() -> CborValue {
    CborValue::Map(vec![(
        CborValue::Text("Recover1".into()),
        CborValue::Null,
    )])
}

fn parse_recover1_response(val: &CborValue) -> Result<[u8; 16]> {
    let inner = cbor_unwrap_ok(val)?;
    let version_bytes = cbor_get_bytes(inner, "version").context("Missing version in Recover1")?;
    if version_bytes.len() != 16 {
        bail!(
            "Invalid version length: {} (expected 16)",
            version_bytes.len()
        );
    }
    let mut version = [0u8; 16];
    version.copy_from_slice(version_bytes);
    Ok(version)
}

async fn phase1_get_version(
    http: &reqwest::Client,
    config: &JuiceboxConfig,
) -> Result<[u8; 16]> {
    let request = build_recover1_cbor();

    // Try each realm until one succeeds
    let mut last_err = None;
    for realm in &config.realms {
        match send_realm_request(http, realm, &request).await {
            Ok(resp) => match parse_recover1_response(&resp) {
                Ok(version) => return Ok(version),
                Err(e) => {
                    tracing::warn!(
                        "Realm {} phase 1 parse error: {e:#}",
                        realm.address
                    );
                    last_err = Some(e);
                }
            },
            Err(e) => {
                tracing::warn!("Realm {} phase 1 request error: {e:#}", realm.address);
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("No realms configured")))
        .context("Phase 1 failed: no realm returned a valid version")
}

// ── Phase 2: OPRF ───────────────────────────────────────────────────

/// Blind the access key for the OPRF protocol.
/// Returns (blinded_point, blinding_factor).
fn oprf_blind(access_key: &[u8; 32]) -> (CompressedRistretto, Scalar) {
    // Hash the access key to a RistrettoPoint using SHA-512 + from_uniform_bytes
    let hash: [u8; 64] = Sha512::digest(access_key).into();
    let input_point = RistrettoPoint::from_uniform_bytes(&hash);

    // Generate a random blinding factor scalar
    // We use rand 0.9 to fill 64 random bytes and reduce mod l,
    // since curve25519-dalek expects rand_core 0.6 for Scalar::random.
    let mut rng_bytes = [0u8; 64];
    rand::fill(&mut rng_bytes);
    let blinding_factor = Scalar::from_bytes_mod_order_wide(&rng_bytes);

    let blinded = input_point * blinding_factor;
    (blinded.compress(), blinding_factor)
}

fn build_recover2_cbor(version: &[u8; 16], blinded_input: &CompressedRistretto) -> CborValue {
    CborValue::Map(vec![(
        CborValue::Text("Recover2".into()),
        CborValue::Map(vec![
            (
                CborValue::Text("version".into()),
                CborValue::Bytes(version.to_vec()),
            ),
            (
                CborValue::Text("oprf_blinded_input".into()),
                CborValue::Bytes(blinded_input.as_bytes().to_vec()),
            ),
        ]),
    )])
}

/// Result from a single realm's Recover2 response.
struct Recover2Result {
    oprf_blinded_result: CompressedRistretto,
    unlock_key_commitment: [u8; 32],
    num_guesses: u16,
    guess_count: u16,
}

fn parse_recover2_response(val: &CborValue) -> Result<Recover2Result> {
    let inner = cbor_unwrap_ok(val)?;

    let blinded_result_bytes = cbor_get_bytes(inner, "oprf_blinded_result")
        .context("Missing oprf_blinded_result")?;
    if blinded_result_bytes.len() != 32 {
        bail!("Invalid oprf_blinded_result length: {}", blinded_result_bytes.len());
    }
    let oprf_blinded_result = CompressedRistretto::from_slice(blinded_result_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid CompressedRistretto: {e}"))?;

    let commitment_bytes = cbor_get_bytes(inner, "unlock_key_commitment")
        .context("Missing unlock_key_commitment")?;
    if commitment_bytes.len() != 32 {
        bail!(
            "Invalid unlock_key_commitment length: {}",
            commitment_bytes.len()
        );
    }
    let mut unlock_key_commitment = [0u8; 32];
    unlock_key_commitment.copy_from_slice(commitment_bytes);

    let num_guesses = cbor_get_u64(inner, "num_guesses").unwrap_or(0) as u16;
    let guess_count = cbor_get_u64(inner, "guess_count").unwrap_or(0) as u16;

    Ok(Recover2Result {
        oprf_blinded_result,
        unlock_key_commitment,
        num_guesses,
        guess_count,
    })
}

/// Lagrange interpolation coefficient for share `i` among the set of `indices`,
/// evaluated at x=0, over the Ristretto scalar field.
fn lagrange_coefficient(i: u64, indices: &[u64]) -> Scalar {
    let xi = Scalar::from(i);
    let mut num = Scalar::ONE;
    let mut den = Scalar::ONE;
    for &j in indices {
        if j == i {
            continue;
        }
        let xj = Scalar::from(j);
        num *= xj;
        den *= xj - xi;
    }
    num * den.invert()
}

/// Lagrange interpolation over RistrettoPoints: reconstruct the value at x=0
/// from shares at the given 1-based indices.
fn lagrange_interpolate_points(shares: &[(u64, RistrettoPoint)]) -> RistrettoPoint {
    let indices: Vec<u64> = shares.iter().map(|(i, _)| *i).collect();
    let mut result = RistrettoPoint::identity();
    for &(i, point) in shares {
        let coeff = lagrange_coefficient(i, &indices);
        result += point * coeff;
    }
    result
}

/// Lagrange interpolation over Scalars: reconstruct the value at x=0
/// from shares at the given 1-based indices.
fn lagrange_interpolate_scalars(shares: &[(u64, Scalar)]) -> Scalar {
    let indices: Vec<u64> = shares.iter().map(|(i, _)| *i).collect();
    let mut result = Scalar::ZERO;
    for &(i, scalar) in shares {
        let coeff = lagrange_coefficient(i, &indices);
        result += scalar * coeff;
    }
    result
}

/// Finalize the OPRF: unblind each realm's result, Lagrange-interpolate, then hash.
/// Returns (unlock_key_commitment[32], unlock_key[32]).
fn oprf_finalize(
    access_key: &[u8; 32],
    blinding_factor: &Scalar,
    realm_results: &[(u64, CompressedRistretto)], // (1-based index, blinded result)
) -> Result<([u8; 32], [u8; 32])> {
    let blinding_inv = blinding_factor.invert();

    // Unblind each realm's result and prepare for Lagrange interpolation
    let mut point_shares: Vec<(u64, RistrettoPoint)> = Vec::new();
    for &(idx, ref compressed) in realm_results {
        let point = compressed
            .decompress()
            .context("Failed to decompress OPRF blinded result")?;
        let unblinded = point * blinding_inv;
        point_shares.push((idx, unblinded));
    }

    // Lagrange interpolation to reconstruct the OPRF output point
    let oprf_result_point = lagrange_interpolate_points(&point_shares);
    let oprf_result_compressed = oprf_result_point.compress();

    // Hash to derive the OPRF output:
    // oprf_output = SHA512("Juicebox_OPRF_2023_1;" || access_key || compressed_result_bytes)
    let mut hasher = Sha512::new();
    hasher.update(b"Juicebox_OPRF_2023_1;");
    hasher.update(access_key);
    hasher.update(oprf_result_compressed.as_bytes());
    let oprf_output: [u8; 64] = hasher.finalize().into();

    // Derive unlock_key_commitment and unlock_key from a second SHA512
    let digest: [u8; 64] = Sha512::digest(&oprf_output).into();
    let mut unlock_key_commitment = [0u8; 32];
    let mut unlock_key = [0u8; 32];
    unlock_key_commitment.copy_from_slice(&digest[..32]);
    unlock_key.copy_from_slice(&digest[32..]);

    Ok((unlock_key_commitment, unlock_key))
}

async fn phase2_oprf(
    http: &reqwest::Client,
    config: &JuiceboxConfig,
    version: &[u8; 16],
    access_key: &[u8; 32],
) -> Result<([u8; 32], [u8; 32])> {
    // Step 1: Blind the access key
    let (blinded_input, blinding_factor) = oprf_blind(access_key);
    let request = build_recover2_cbor(version, &blinded_input);

    // Step 2: Send to all realms, collect responses
    let threshold = config.recover_threshold as usize;
    let mut realm_results: Vec<(u64, CompressedRistretto)> = Vec::new();
    let mut commitment: Option<[u8; 32]> = None;

    for (realm_idx, realm) in config.realms.iter().enumerate() {
        let one_based_idx = (realm_idx + 1) as u64;

        match send_realm_request(http, realm, &request).await {
            Ok(resp) => match parse_recover2_response(&resp) {
                Ok(r2) => {
                    tracing::debug!(
                        "Realm {} phase 2 OK: guesses {}/{}",
                        realm.address,
                        r2.guess_count,
                        r2.num_guesses
                    );
                    if commitment.is_none() {
                        commitment = Some(r2.unlock_key_commitment);
                    }
                    realm_results.push((one_based_idx, r2.oprf_blinded_result));
                }
                Err(e) => {
                    tracing::warn!("Realm {} phase 2 parse error: {e:#}", realm.address);
                }
            },
            Err(e) => {
                tracing::warn!("Realm {} phase 2 request error: {e:#}", realm.address);
            }
        }

        if realm_results.len() >= threshold {
            break;
        }
    }

    if realm_results.len() < threshold {
        bail!(
            "Phase 2 failed: only {}/{} realms responded",
            realm_results.len(),
            threshold
        );
    }

    // Step 3: Finalize OPRF (unblind + Lagrange + hash)
    let (computed_commitment, unlock_key) =
        oprf_finalize(access_key, &blinding_factor, &realm_results)?;

    // Step 4: Verify unlock_key_commitment matches
    if let Some(expected) = commitment {
        if computed_commitment != expected {
            bail!("Unlock key commitment mismatch: OPRF verification failed (wrong PIN?)");
        }
    }

    Ok((unlock_key, computed_commitment))
}

// ── Phase 3: Recover Secret ─────────────────────────────────────────

/// Compute the unlock_key_tag for a realm.
/// BLAKE2s-MAC-128(key=unlock_key, data = to_be4(len(label)) || label || to_be4(len(realm_id)) || realm_id)
fn compute_unlock_key_tag(unlock_key: &[u8; 32], realm_id: &[u8; 16]) -> Result<[u8; 16]> {
    let label = b"Unlock Key Tag";
    let mut data = Vec::new();
    data.extend_from_slice(&to_be4(label.len()));
    data.extend_from_slice(label);
    data.extend_from_slice(&to_be4(realm_id.len()));
    data.extend_from_slice(realm_id);

    let mut mac = <Blake2sMac128 as KeyInit>::new_from_slice(unlock_key)
        .map_err(|e| anyhow::anyhow!("BLAKE2s key: {e}"))?;
    mac.update(&data);
    let result = mac.finalize().into_bytes();

    let mut tag = [0u8; 16];
    tag.copy_from_slice(&result);
    Ok(tag)
}

fn build_recover3_cbor(version: &[u8; 16], unlock_key_tag: &[u8; 16]) -> CborValue {
    CborValue::Map(vec![(
        CborValue::Text("Recover3".into()),
        CborValue::Map(vec![
            (
                CborValue::Text("version".into()),
                CborValue::Bytes(version.to_vec()),
            ),
            (
                CborValue::Text("unlock_key_tag".into()),
                CborValue::Bytes(unlock_key_tag.to_vec()),
            ),
        ]),
    )])
}

/// Result from a single realm's Recover3 response.
struct Recover3Result {
    encryption_key_scalar_share: Scalar,
    encrypted_secret: Vec<u8>,
    encrypted_secret_commitment: [u8; 16],
}

fn parse_recover3_response(val: &CborValue) -> Result<Recover3Result> {
    let inner = cbor_unwrap_ok(val)?;

    let scalar_bytes =
        cbor_get_bytes(inner, "encryption_key_scalar_share").context("Missing scalar share")?;
    if scalar_bytes.len() != 32 {
        bail!("Invalid scalar share length: {}", scalar_bytes.len());
    }
    let mut scalar_arr = [0u8; 32];
    scalar_arr.copy_from_slice(scalar_bytes);
    // Scalars in curve25519-dalek are little-endian canonical
    let encryption_key_scalar_share = Scalar::from_canonical_bytes(scalar_arr)
        .into_option()
        .context("Invalid scalar: not canonical")?;

    let encrypted_secret =
        cbor_get_bytes(inner, "encrypted_secret").context("Missing encrypted_secret")?;

    let commitment_bytes = cbor_get_bytes(inner, "encrypted_secret_commitment")
        .context("Missing encrypted_secret_commitment")?;
    if commitment_bytes.len() != 16 {
        bail!(
            "Invalid encrypted_secret_commitment length: {}",
            commitment_bytes.len()
        );
    }
    let mut encrypted_secret_commitment = [0u8; 16];
    encrypted_secret_commitment.copy_from_slice(commitment_bytes);

    Ok(Recover3Result {
        encryption_key_scalar_share,
        encrypted_secret: encrypted_secret.clone(),
        encrypted_secret_commitment,
    })
}

/// Derive the final encryption key from the seed and reconstructed scalar.
/// encryption_key = BLAKE2s-MAC-256(key=encryption_key_seed, data=label_data || scalar_bytes)
fn derive_encryption_key(
    encryption_key_seed: &[u8; 32],
    encryption_key_scalar: &Scalar,
) -> Result<[u8; 32]> {
    let label = b"User Secret Encryption Key";
    let scalar_bytes = encryption_key_scalar.to_bytes();

    let mut data = Vec::new();
    data.extend_from_slice(&to_be4(label.len()));
    data.extend_from_slice(label);
    data.extend_from_slice(&to_be4(scalar_bytes.len()));
    data.extend_from_slice(&scalar_bytes);

    let mut mac = <Blake2sMac256 as KeyInit>::new_from_slice(encryption_key_seed)
        .map_err(|e| anyhow::anyhow!("BLAKE2s-256 key: {e}"))?;
    mac.update(&data);
    let result = mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    Ok(key)
}

/// Decrypt the secret using ChaCha20Poly1305.
fn decrypt_secret(encryption_key: &[u8; 32], encrypted_secret: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(encryption_key.into());
    let nonce = chacha20poly1305::Nonce::from([0u8; 12]);

    let padded_secret = cipher
        .decrypt(&nonce, encrypted_secret)
        .map_err(|e| anyhow::anyhow!("ChaCha20Poly1305 decrypt failed: {e}"))?;

    if padded_secret.is_empty() {
        bail!("Decrypted secret is empty");
    }

    // First byte is the actual secret length, rest is the secret + padding
    let secret_len = padded_secret[0] as usize;
    if secret_len + 1 > padded_secret.len() {
        bail!(
            "Invalid secret length prefix: {} but padded len is {}",
            secret_len,
            padded_secret.len()
        );
    }

    Ok(padded_secret[1..1 + secret_len].to_vec())
}

async fn phase3_recover(
    http: &reqwest::Client,
    config: &JuiceboxConfig,
    version: &[u8; 16],
    unlock_key: &[u8; 32],
    encryption_key_seed: &[u8; 32],
) -> Result<Vec<u8>> {
    let threshold = config.recover_threshold as usize;

    // Step 1: Send Recover3 to each realm
    let mut scalar_shares: Vec<(u64, Scalar)> = Vec::new();
    let mut encrypted_secret: Option<Vec<u8>> = None;

    for (realm_idx, realm) in config.realms.iter().enumerate() {
        let one_based_idx = (realm_idx + 1) as u64;
        let unlock_key_tag = compute_unlock_key_tag(unlock_key, &realm.id)?;
        let request = build_recover3_cbor(version, &unlock_key_tag);

        match send_realm_request(http, realm, &request).await {
            Ok(resp) => match parse_recover3_response(&resp) {
                Ok(r3) => {
                    tracing::debug!("Realm {} phase 3 OK", realm.address);
                    scalar_shares.push((one_based_idx, r3.encryption_key_scalar_share));
                    if encrypted_secret.is_none() {
                        encrypted_secret = Some(r3.encrypted_secret);
                    }
                }
                Err(e) => {
                    tracing::warn!("Realm {} phase 3 parse error: {e:#}", realm.address);
                }
            },
            Err(e) => {
                tracing::warn!("Realm {} phase 3 request error: {e:#}", realm.address);
            }
        }

        if scalar_shares.len() >= threshold {
            break;
        }
    }

    if scalar_shares.len() < threshold {
        bail!(
            "Phase 3 failed: only {}/{} realms responded",
            scalar_shares.len(),
            threshold
        );
    }

    let encrypted_secret = encrypted_secret.context("No encrypted_secret received")?;

    // Step 2: Reconstruct encryption_key_scalar via Lagrange interpolation
    let encryption_key_scalar = lagrange_interpolate_scalars(&scalar_shares);

    // Step 3: Derive encryption key
    let encryption_key = derive_encryption_key(encryption_key_seed, &encryption_key_scalar)?;

    // Step 4: Decrypt the secret
    decrypt_secret(&encryption_key, &encrypted_secret)
}

// ── Main entry point ────────────────────────────────────────────────

/// Attempt to recover the user's private key using their PIN.
/// This is the main entry point for E2EE DM decryption.
///
/// Implements the full 3-phase Juicebox distributed recovery protocol:
/// 1. Get registration version from realms
/// 2. OPRF exchange (blinded PIN verification with threshold reconstruction)
/// 3. Recover encrypted secret shares and reconstruct the private key
pub async fn recover_private_key(
    http: &reqwest::Client,
    config: &JuiceboxConfig,
    pin: &str,
    user_id: &str,
) -> Result<Vec<u8>> {
    tracing::info!(
        "Starting Juicebox recovery: {} realms, threshold {}",
        config.realms.len(),
        config.recover_threshold
    );

    // Phase 1: Get version from realms
    let version = phase1_get_version(http, config)
        .await
        .context("Phase 1 (get version) failed")?;
    tracing::info!("Phase 1 complete: got version {}", hex::encode(&version));

    // Stretch PIN with Argon2id
    let (access_key, encryption_key_seed) = stretch_pin(pin, &version, user_id.as_bytes())?;
    tracing::info!("PIN stretched with Argon2id");

    // Phase 2: OPRF exchange
    let (unlock_key, _commitment) = phase2_oprf(http, config, &version, &access_key)
        .await
        .context("Phase 2 (OPRF) failed")?;
    tracing::info!("Phase 2 complete: OPRF verified");

    // Phase 3: Recover shares and reconstruct secret
    let secret = phase3_recover(http, config, &version, &unlock_key, &encryption_key_seed)
        .await
        .context("Phase 3 (recover) failed")?;
    tracing::info!("Phase 3 complete: secret recovered ({} bytes)", secret.len());

    Ok(secret)
}
