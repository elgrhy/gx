//! Cryptographic primitives: HMAC, constant-time compare, secure randomness,
//! Ed25519 signatures, and HS256-only JWT.
//!
//! All actual cryptographic math (hashing, MAC, elliptic-curve signing, CSPRNG)
//! is delegated to audited crates (`hmac`+`sha2`, `subtle`, `getrandom`,
//! `ed25519-dalek`, `jsonwebtoken`) — this file only validates arguments and
//! converts between GX `Value` and each crate's types. No cryptographic
//! algorithm is implemented by hand here.

use super::builtins_json::{gx_value_to_json, json_to_gx_value};
use super::util::{arg_str, arg_str_checked};
use super::Signal;
use crate::value::Value;
use std::collections::HashMap;

// ── Production input-size limits ──────────────────────────────────────────────
//
// Every function here that touches attacker-reachable input (HMAC message,
// Ed25519 message/signature/key, JWT token) enforces one of these caps before
// doing any real work (hex/base64 decode, hashing) — without them, a
// webhook handler that pipes an unbounded request body straight into
// `hmac_sha256`/`ed25519_verify`/`jwt_verify` has no protection against a
// caller sending gigabytes of input just to burn CPU/memory.
//
/// HMAC keys are short, high-entropy secrets — never legitimately large.
const MAX_HMAC_KEY_LEN: usize = 4 * 1024; // 4 KiB
/// Generic message/payload cap shared by HMAC and Ed25519 sign/verify —
/// generous enough for real webhook bodies, bounded enough to cap abuse.
const MAX_MESSAGE_LEN: usize = 10 * 1024 * 1024; // 10 MiB
/// Ed25519 keys are always 32 raw bytes (64 hex chars); this cap is checked
/// before hex-decoding so an oversized string is rejected without ever
/// allocating a decode buffer for it.
const MAX_ED25519_KEY_HEX_LEN: usize = 256;
/// Ed25519 signatures are always 64 raw bytes (128 hex chars); same rationale.
const MAX_ED25519_SIG_HEX_LEN: usize = 512;
/// Real-world HS256 JWTs are almost always well under 2 KiB; 8 KiB matches
/// common HTTP header-size limits (e.g. nginx's default) and leaves generous
/// headroom while still bounding abuse.
const MAX_JWT_TOKEN_LEN: usize = 8 * 1024; // 8 KiB
/// Cap on the serialized JSON size of a `jwt_sign` payload, keeping the
/// resulting (base64-inflated) token comfortably under `MAX_JWT_TOKEN_LEN`.
const MAX_JWT_PAYLOAD_JSON_LEN: usize = 8 * 1024; // 8 KiB
/// JWT secrets are short, high-entropy values — never legitimately large.
const MAX_JWT_SECRET_LEN: usize = 1024; // 1 KiB
/// Minimum HS256 secret length: 256 bits, the standard floor recommended for
/// HMAC-SHA256 keys (RFC 2104 / NIST SP 800-107) so the secret can't be
/// brute-forced offline once an attacker has a single valid token.
const MIN_JWT_SECRET_LEN: usize = 32;
/// `secure_random`'s own output-size cap (unchanged from the original
/// implementation, just named here for consistency with the limits above).
const MAX_SECURE_RANDOM_LEN: usize = 1024 * 1024; // 1 MiB

const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

/// Lookup-table hex encoder: one preallocated buffer, no per-byte heap
/// allocation (the previous `.map(|b| format!(...)).collect()` implementation
/// allocated a separate String for every single byte).
pub(super) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX_CHARS[(b >> 4) as usize] as char);
        out.push(HEX_CHARS[(b & 0x0f) as usize] as char);
    }
    out
}

pub(super) fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex string".into());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        let pair = std::str::from_utf8(chunk).map_err(|_| "invalid hex string".to_string())?;
        let byte = u8::from_str_radix(pair, 16).map_err(|_| "invalid hex character".to_string())?;
        out.push(byte);
    }
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn crypto_builtin(name: &str, _args: &[Value]) -> Result<Value, Signal> {
    Err(Signal::Error(format!(
        "{} is not available in the WASM playground",
        name
    )))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn crypto_builtin(name: &str, args: &[Value]) -> Result<Value, Signal> {
    match name {
        "hmac_sha256" => {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let key = arg_str_checked(args, 0, "hmac_sha256", MAX_HMAC_KEY_LEN)?;
            let msg = arg_str_checked(args, 1, "hmac_sha256", MAX_MESSAGE_LEN)?;
            let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key.as_bytes())
                .map_err(|e| Signal::Error(format!("hmac_sha256: invalid key: {}", e)))?;
            mac.update(msg.as_bytes());
            Ok(Value::Str(hex_encode(&mac.finalize().into_bytes())))
        }
        "hmac_sha512" => {
            use hmac::{Hmac, Mac};
            use sha2::Sha512;
            let key = arg_str_checked(args, 0, "hmac_sha512", MAX_HMAC_KEY_LEN)?;
            let msg = arg_str_checked(args, 1, "hmac_sha512", MAX_MESSAGE_LEN)?;
            let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(key.as_bytes())
                .map_err(|e| Signal::Error(format!("hmac_sha512: invalid key: {}", e)))?;
            mac.update(msg.as_bytes());
            Ok(Value::Str(hex_encode(&mac.finalize().into_bytes())))
        }
        "secure_compare" => {
            use subtle::ConstantTimeEq;
            let a = arg_str(args, 0, "secure_compare")?;
            let b = arg_str(args, 1, "secure_compare")?;
            let eq: bool = a.as_bytes().ct_eq(b.as_bytes()).into();
            Ok(Value::Bool(eq))
        }
        "secure_random" => {
            let len = args
                .first()
                .and_then(|v| v.as_number())
                .ok_or_else(|| Signal::Error("secure_random requires a numeric length".into()))?;
            if !(0.0..=MAX_SECURE_RANDOM_LEN as f64).contains(&len) {
                return Err(Signal::Error(format!(
                    "secure_random: length must be between 0 and {}",
                    MAX_SECURE_RANDOM_LEN
                )));
            }
            let mut buf = vec![0u8; len as usize];
            getrandom::getrandom(&mut buf)
                .map_err(|e| Signal::Error(format!("secure_random: {}", e)))?;
            Ok(Value::Str(hex_encode(&buf)))
        }
        "ed25519_generate_keypair" => {
            use ed25519_dalek::SigningKey;
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed)
                .map_err(|e| Signal::Error(format!("ed25519_generate_keypair: {}", e)))?;
            let signing_key = SigningKey::from_bytes(&seed);
            let verifying_key = signing_key.verifying_key();
            let mut obj = HashMap::new();
            obj.insert(
                "public_key".into(),
                Value::Str(hex_encode(&verifying_key.to_bytes())),
            );
            obj.insert(
                "private_key".into(),
                Value::Str(hex_encode(&signing_key.to_bytes())),
            );
            Ok(Value::Object(obj))
        }
        "ed25519_sign" => {
            use ed25519_dalek::{Signer, SigningKey};
            let private_key = arg_str_checked(args, 0, "ed25519_sign", MAX_ED25519_KEY_HEX_LEN)?;
            let message = arg_str_checked(args, 1, "ed25519_sign", MAX_MESSAGE_LEN)?;
            let seed_bytes = hex_decode(&private_key).map_err(|e| {
                Signal::Error(format!("ed25519_sign: malformed private key: {}", e))
            })?;
            let seed: [u8; 32] = seed_bytes.try_into().map_err(|_| {
                Signal::Error(
                    "ed25519_sign: private key must be exactly 32 bytes (64 hex chars)".into(),
                )
            })?;
            let signing_key = SigningKey::from_bytes(&seed);
            let signature = signing_key.sign(message.as_bytes());
            Ok(Value::Str(hex_encode(&signature.to_bytes())))
        }
        "ed25519_verify" => {
            use ed25519_dalek::{Signature, Verifier, VerifyingKey};
            // Extracted as borrowed &str, never cloned: none of these need
            // to outlive this match arm, so there's nothing to gain by
            // copying them (and, for the oversized case below, real cost to
            // avoid — see the length check that runs before anything else).
            let public_key = args.first().and_then(|v| v.as_str()).ok_or_else(|| {
                Signal::Error("ed25519_verify requires a string argument at position 1".into())
            })?;
            let message = args.get(1).and_then(|v| v.as_str()).ok_or_else(|| {
                Signal::Error("ed25519_verify requires a string argument at position 2".into())
            })?;
            let signature = args.get(2).and_then(|v| v.as_str()).ok_or_else(|| {
                Signal::Error("ed25519_verify requires a string argument at position 3".into())
            })?;
            // Verification is a predicate over untrusted input: any malformed
            // key/signature simply fails the check (returns false) rather than
            // raising a runtime error, so a forged/garbage webhook payload can
            // never crash or short-circuit a GX handler that forgets try/catch.
            let ok = (|| -> Option<bool> {
                if public_key.len() > MAX_ED25519_KEY_HEX_LEN
                    || signature.len() > MAX_ED25519_SIG_HEX_LEN
                    || message.len() > MAX_MESSAGE_LEN
                {
                    // Oversized input fails the check like any other malformed
                    // input — checked before hex-decoding so an attacker can't
                    // force an allocation proportional to a huge garbage string.
                    return None;
                }
                let pub_bytes = hex_decode(public_key).ok()?;
                let pub_arr: [u8; 32] = pub_bytes.try_into().ok()?;
                let verifying_key = VerifyingKey::from_bytes(&pub_arr).ok()?;
                let sig_bytes = hex_decode(signature).ok()?;
                let sig_arr: [u8; 64] = sig_bytes.try_into().ok()?;
                let sig = Signature::from_bytes(&sig_arr);
                Some(verifying_key.verify(message.as_bytes(), &sig).is_ok())
            })()
            .unwrap_or(false);
            Ok(Value::Bool(ok))
        }
        "jwt_sign" => jwt_sign_impl(args),
        "jwt_verify" => jwt_verify_impl(args),
        _ => Err(Signal::Error(format!("Unknown crypto builtin: {}", name))),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(not(target_arch = "wasm32"))]
fn jwt_sign_impl(args: &[Value]) -> Result<Value, Signal> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

    let payload = args
        .first()
        .ok_or_else(|| Signal::Error("jwt_sign requires a payload object".into()))?;
    let secret = arg_str_checked(args, 1, "jwt_sign", MAX_JWT_SECRET_LEN)?;
    if secret.len() < MIN_JWT_SECRET_LEN {
        return Err(Signal::Error(format!(
            "jwt_sign: secret must be at least {} bytes for HS256 (got {}) — \
             a short secret can be brute-forced offline once an attacker has one valid token",
            MIN_JWT_SECRET_LEN,
            secret.len()
        )));
    }
    if !matches!(payload, Value::Object(_)) {
        return Err(Signal::Error("jwt_sign: payload must be an object".into()));
    }
    // Fast-reject pass: estimate the serialized size directly from the GX
    // `Value` tree, before ever calling `gx_value_to_json` (which allocates
    // a full parallel `serde_json::Value` tree) or `serde_json::to_string`
    // (which serializes it just to measure the result). A payload that's
    // clearly oversized by this estimate is certainly oversized for real —
    // see `estimate_json_size_within`'s doc comment for why.
    if super::builtins_json::estimate_json_size_within(payload, MAX_JWT_PAYLOAD_JSON_LEN).is_none()
    {
        return Err(Signal::Error(format!(
            "jwt_sign: payload exceeds the maximum serialized size of {} bytes",
            MAX_JWT_PAYLOAD_JSON_LEN
        )));
    }
    let claims = gx_value_to_json(payload);
    // The estimate above can under-count (JSON string escaping only ever
    // grows a string's encoded size), so anything that passed it still needs
    // this precise check — this is what actually enforces the limit.
    let payload_json_len = serde_json::to_string(&claims)
        .map(|s| s.len())
        .unwrap_or(usize::MAX); // fail closed: treat a serialization error as oversized
    if payload_json_len > MAX_JWT_PAYLOAD_JSON_LEN {
        return Err(Signal::Error(format!(
            "jwt_sign: payload exceeds the maximum serialized size of {} bytes (got {})",
            MAX_JWT_PAYLOAD_JSON_LEN, payload_json_len
        )));
    }

    let header = Header::new(Algorithm::HS256);
    let key = EncodingKey::from_secret(secret.as_bytes());
    encode(&header, &claims, &key)
        .map(Value::Str)
        .map_err(|e| Signal::Error(format!("jwt_sign: {}", e)))
}

/// Build the `{valid:false, payload:null, header, reason}` result object.
#[cfg(not(target_arch = "wasm32"))]
fn jwt_invalid(header: Value, reason: String) -> Value {
    let mut obj = HashMap::new();
    obj.insert("valid".into(), Value::Bool(false));
    obj.insert("payload".into(), Value::Null);
    obj.insert("header".into(), header);
    obj.insert("reason".into(), Value::Str(reason));
    Value::Object(obj)
}

#[cfg(not(target_arch = "wasm32"))]
fn jwt_verify_impl(args: &[Value]) -> Result<Value, Signal> {
    use jsonwebtoken::errors::ErrorKind;
    use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};

    // Length is checked on the borrowed &str, before cloning either one —
    // jwt_verify sits directly in front of untrusted network input (a bearer
    // token, a webhook payload), so an oversized value must be rejected
    // without ever copying it, let alone base64/JSON-decoding it.
    let token_raw = args.first().and_then(|v| v.as_str()).ok_or_else(|| {
        Signal::Error("jwt_verify requires a string argument at position 1".into())
    })?;
    let secret_raw = args.get(1).and_then(|v| v.as_str()).ok_or_else(|| {
        Signal::Error("jwt_verify requires a string argument at position 2".into())
    })?;
    if token_raw.len() > MAX_JWT_TOKEN_LEN || secret_raw.len() > MAX_JWT_SECRET_LEN {
        return Ok(jwt_invalid(Value::Null, "input_too_large".to_string()));
    }
    let token = token_raw.to_string();
    let secret = secret_raw.to_string();

    // decode_header() only base64/JSON-decodes the header segment — it does
    // NOT verify the signature. It is safe to call on fully untrusted input:
    // jsonwebtoken's `Algorithm` enum has no "none" variant, so a token with
    // `"alg":"none"` (or any algorithm name jsonwebtoken doesn't recognize)
    // fails to parse here and falls into the malformed-token branch below.
    let header = match decode_header(&token) {
        Ok(h) => h,
        Err(e) => return Ok(jwt_invalid(Value::Null, format!("malformed token: {}", e))),
    };
    let header_gx = {
        let mut m = HashMap::new();
        m.insert("alg".into(), Value::Str(format!("{:?}", header.alg)));
        m.insert(
            "typ".into(),
            header.typ.clone().map(Value::Str).unwrap_or(Value::Null),
        );
        Value::Object(m)
    };

    // Explicit algorithm allowlist: reject every algorithm except HS256, even
    // ones jsonwebtoken itself supports (RS256, ES256, EdDSA, ...). This is
    // the defense against algorithm-confusion attacks — never let the token's
    // own header pick a verification strategy outside what the caller allows.
    if header.alg != Algorithm::HS256 {
        return Ok(jwt_invalid(
            header_gx,
            format!(
                "unsupported algorithm '{:?}': only HS256 is supported",
                header.alg
            ),
        ));
    }

    // Checked here (after the algorithm is confirmed HS256, before the secret
    // is actually used to verify anything) rather than throwing: jwt_verify
    // never raises for verification-time concerns, it always reports why in
    // `reason` so a caller doesn't need try/catch around every call.
    if secret.len() < MIN_JWT_SECRET_LEN {
        return Ok(jwt_invalid(
            header_gx,
            format!(
                "secret must be at least {} bytes for HS256 verification",
                MIN_JWT_SECRET_LEN
            ),
        ));
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_nbf = true; // opt in: jsonwebtoken defaults this to false
    let decoding_key = DecodingKey::from_secret(secret.as_bytes());

    match decode::<serde_json::Value>(&token, &decoding_key, &validation) {
        Ok(data) => {
            // jsonwebtoken doesn't validate `iat` itself — a token claiming to
            // have been issued in the future is suspicious, so check it here.
            if let Some(iat) = data.claims.get("iat").and_then(|v| v.as_i64()) {
                if iat > now_secs() + validation.leeway as i64 {
                    return Ok(jwt_invalid(
                        header_gx,
                        "token is not valid yet (iat is in the future)".into(),
                    ));
                }
            }
            let mut obj = HashMap::new();
            obj.insert("valid".into(), Value::Bool(true));
            obj.insert("payload".into(), json_to_gx_value(&data.claims));
            obj.insert("header".into(), header_gx);
            obj.insert("reason".into(), Value::Null);
            Ok(Value::Object(obj))
        }
        Err(e) => {
            let reason = match e.kind() {
                ErrorKind::ExpiredSignature => "token has expired".to_string(),
                ErrorKind::ImmatureSignature => "token is not yet valid (nbf)".to_string(),
                ErrorKind::InvalidSignature => "invalid signature".to_string(),
                ErrorKind::MissingRequiredClaim(c) => format!("missing required claim '{}'", c),
                ErrorKind::InvalidAlgorithm => "unsupported algorithm".to_string(),
                _ => format!("invalid token: {}", e),
            };
            Ok(jwt_invalid(header_gx, reason))
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::Str(v.to_string())
    }

    /// A valid (>= MIN_JWT_SECRET_LEN) test secret — real JWT secrets must be
    /// at least 32 bytes, so plain words like "secret" no longer pass.
    fn long_secret() -> Value {
        s(&"a".repeat(MIN_JWT_SECRET_LEN))
    }

    /// A second, distinct >= 32-byte secret, for wrong-secret tests.
    fn other_long_secret() -> Value {
        s(&"b".repeat(MIN_JWT_SECRET_LEN))
    }

    #[test]
    fn hmac_sha256_known_vector() {
        // RFC 4231 test case 1
        let key = Value::Str("\u{0b}".repeat(20));
        let args = vec![key, s("Hi There")];
        let result = crypto_builtin("hmac_sha256", &args).unwrap();
        assert_eq!(
            result,
            Value::Str(
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7".to_string()
            ),
            "hmac_sha256(20-byte 0x0b key, \"Hi There\") must match RFC 4231 test case 1"
        );
    }

    #[test]
    fn hmac_is_deterministic_and_key_sensitive() {
        let a = crypto_builtin("hmac_sha256", &[s("key1"), s("message")]).unwrap();
        let b = crypto_builtin("hmac_sha256", &[s("key1"), s("message")]).unwrap();
        let c = crypto_builtin("hmac_sha256", &[s("key2"), s("message")]).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hmac_sha512_produces_128_hex_chars() {
        let result = crypto_builtin("hmac_sha512", &[s("key"), s("msg")]).unwrap();
        assert_eq!(result.as_str().unwrap().len(), 128);
    }

    #[test]
    fn secure_compare_equal_and_unequal() {
        assert_eq!(
            crypto_builtin("secure_compare", &[s("abc"), s("abc")]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            crypto_builtin("secure_compare", &[s("abc"), s("abd")]).unwrap(),
            Value::Bool(false)
        );
        // Different lengths must not panic and must be unequal.
        assert_eq!(
            crypto_builtin("secure_compare", &[s("abc"), s("abcd")]).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn secure_random_length_and_uniqueness() {
        let r1 = crypto_builtin("secure_random", &[Value::Number(16.0)]).unwrap();
        let r2 = crypto_builtin("secure_random", &[Value::Number(16.0)]).unwrap();
        assert_eq!(r1.as_str().unwrap().len(), 32); // 16 bytes -> 32 hex chars
        assert_ne!(r1, r2);
    }

    #[test]
    fn secure_random_rejects_oversized_length() {
        let err = crypto_builtin("secure_random", &[Value::Number(1e9)]);
        assert!(err.is_err());
    }

    #[test]
    fn ed25519_roundtrip_sign_and_verify() {
        let keypair = crypto_builtin("ed25519_generate_keypair", &[]).unwrap();
        let (public_key, private_key) = match &keypair {
            Value::Object(m) => (m["public_key"].clone(), m["private_key"].clone()),
            _ => panic!("expected object"),
        };
        let sig = crypto_builtin("ed25519_sign", &[private_key, s("hello world")]).unwrap();
        let ok = crypto_builtin("ed25519_verify", &[public_key, s("hello world"), sig]).unwrap();
        assert_eq!(ok, Value::Bool(true));
    }

    #[test]
    fn ed25519_verify_rejects_tampered_message() {
        let keypair = crypto_builtin("ed25519_generate_keypair", &[]).unwrap();
        let (public_key, private_key) = match &keypair {
            Value::Object(m) => (m["public_key"].clone(), m["private_key"].clone()),
            _ => panic!("expected object"),
        };
        let sig = crypto_builtin("ed25519_sign", &[private_key, s("original")]).unwrap();
        let ok = crypto_builtin("ed25519_verify", &[public_key, s("tampered"), sig]).unwrap();
        assert_eq!(ok, Value::Bool(false));
    }

    #[test]
    fn ed25519_verify_never_panics_on_malformed_input() {
        let cases = [
            ("", "", ""),
            ("zz", "msg", "sig"),
            ("aabb", "msg", "aabb"),
            ("deadbeef", "msg", "deadbeef"),
        ];
        for (pk, msg, sig) in cases {
            let result = crypto_builtin("ed25519_verify", &[s(pk), s(msg), s(sig)]).unwrap();
            assert_eq!(result, Value::Bool(false));
        }
    }

    #[test]
    fn ed25519_sign_rejects_malformed_key() {
        let err = crypto_builtin("ed25519_sign", &[s("not-hex!!"), s("msg")]);
        assert!(err.is_err());
        let err2 = crypto_builtin("ed25519_sign", &[s("aabb"), s("msg")]); // too short
        assert!(err2.is_err());
    }

    #[test]
    fn jwt_roundtrip_valid() {
        let mut payload = HashMap::new();
        payload.insert("sub".to_string(), s("user-123"));
        payload.insert("exp".to_string(), Value::Number((now_secs() + 3600) as f64));
        let token = crypto_builtin("jwt_sign", &[Value::Object(payload), long_secret()]).unwrap();
        let result = crypto_builtin("jwt_verify", &[token, long_secret()]).unwrap();
        let obj = match result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["valid"], Value::Bool(true));
        assert_eq!(obj["reason"], Value::Null);
    }

    #[test]
    fn jwt_wrong_secret_is_invalid() {
        let mut payload = HashMap::new();
        payload.insert("exp".to_string(), Value::Number((now_secs() + 3600) as f64));
        let token = crypto_builtin("jwt_sign", &[Value::Object(payload), long_secret()]).unwrap();
        let result = crypto_builtin("jwt_verify", &[token, other_long_secret()]).unwrap();
        let obj = match result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["valid"], Value::Bool(false));
    }

    #[test]
    fn jwt_expired_token_is_invalid() {
        let mut payload = HashMap::new();
        payload.insert("exp".to_string(), Value::Number((now_secs() - 3600) as f64));
        let token = crypto_builtin("jwt_sign", &[Value::Object(payload), long_secret()]).unwrap();
        let result = crypto_builtin("jwt_verify", &[token, long_secret()]).unwrap();
        let obj = match result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["valid"], Value::Bool(false));
        assert_eq!(obj["reason"], Value::Str("token has expired".to_string()));
    }

    /// Base64url (no padding) encode, built on the project's own base64 —
    /// only used here to hand-craft a malicious token for the test below.
    fn b64url(data: &str) -> String {
        crate::interpreter::builtins_base64::base64_encode(data.as_bytes())
            .replace('+', "-")
            .replace('/', "_")
            .trim_end_matches('=')
            .to_string()
    }

    #[test]
    fn jwt_rejects_alg_none() {
        // Manually construct a `{"alg":"none","typ":"JWT"}` token with an
        // empty signature segment — the classic alg-confusion forgery.
        let header = b64url(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = b64url(r#"{"sub":"attacker"}"#);
        let token = format!("{}.{}.", header, payload);
        let result = crypto_builtin("jwt_verify", &[s(&token), s("secret")]).unwrap();
        let obj = match result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["valid"], Value::Bool(false));
    }

    #[test]
    fn jwt_rejects_non_hs256_algorithm() {
        // A syntactically valid HS384 header/signature triple (jsonwebtoken
        // itself supports HS384 as an algorithm) must still be rejected by
        // GX's explicit HS256-only allowlist.
        let mut payload = HashMap::new();
        payload.insert("exp".to_string(), Value::Number((now_secs() + 3600) as f64));
        let claims = gx_value_to_json(&Value::Object(payload));
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS384);
        let key = jsonwebtoken::EncodingKey::from_secret(b"secret");
        let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
        let result = crypto_builtin("jwt_verify", &[s(&token), s("secret")]).unwrap();
        let obj = match result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["valid"], Value::Bool(false));
        let reason = obj["reason"].as_str().unwrap();
        assert!(reason.contains("HS256"));
    }

    #[test]
    fn jwt_malformed_token_never_panics() {
        for bad in ["", "not-a-jwt", "a.b", "a.b.c.d", "....."] {
            let result = crypto_builtin("jwt_verify", &[s(bad), s("secret")]).unwrap();
            match result {
                Value::Object(m) => assert_eq!(m["valid"], Value::Bool(false)),
                _ => panic!("expected object"),
            }
        }
    }

    #[test]
    fn jwt_sign_rejects_non_object_payload() {
        let err = crypto_builtin("jwt_sign", &[s("not an object"), s("secret")]);
        assert!(err.is_err());
    }

    // ── Input-size limits ──────────────────────────────────────────────────────

    #[test]
    fn hmac_sha256_rejects_oversized_message() {
        let huge = "x".repeat(MAX_MESSAGE_LEN + 1);
        let err = crypto_builtin("hmac_sha256", &[s("key"), s(&huge)]);
        assert!(err.is_err());
    }

    #[test]
    fn hmac_sha256_rejects_oversized_key() {
        let huge_key = "k".repeat(MAX_HMAC_KEY_LEN + 1);
        let err = crypto_builtin("hmac_sha256", &[s(&huge_key), s("msg")]);
        assert!(err.is_err());
    }

    #[test]
    fn ed25519_sign_rejects_oversized_message() {
        let keypair = crypto_builtin("ed25519_generate_keypair", &[]).unwrap();
        let private_key = match &keypair {
            Value::Object(m) => m["private_key"].clone(),
            _ => panic!("expected object"),
        };
        let huge = "x".repeat(MAX_MESSAGE_LEN + 1);
        let err = crypto_builtin("ed25519_sign", &[private_key, s(&huge)]);
        assert!(err.is_err());
    }

    #[test]
    fn ed25519_verify_rejects_oversized_signature_as_false_not_error() {
        let keypair = crypto_builtin("ed25519_generate_keypair", &[]).unwrap();
        let public_key = match &keypair {
            Value::Object(m) => m["public_key"].clone(),
            _ => panic!("expected object"),
        };
        // An oversized signature must fail the check (false), never throw —
        // ed25519_verify is a predicate over untrusted input.
        let huge_sig = "ab".repeat(MAX_ED25519_SIG_HEX_LEN); // valid hex, way too long
        let result =
            crypto_builtin("ed25519_verify", &[public_key, s("msg"), s(&huge_sig)]).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn ed25519_verify_rejects_oversized_message_as_false_not_error() {
        let keypair = crypto_builtin("ed25519_generate_keypair", &[]).unwrap();
        let (public_key, private_key) = match &keypair {
            Value::Object(m) => (m["public_key"].clone(), m["private_key"].clone()),
            _ => panic!("expected object"),
        };
        // A real signature over a small message — the point is that
        // ed25519_verify rejects the oversized *message* argument (as false,
        // not an error) regardless of whether the signature itself is valid.
        let sig = crypto_builtin("ed25519_sign", &[private_key, s("small message")]).unwrap();
        let huge_message = "x".repeat(MAX_MESSAGE_LEN + 1);
        let result =
            crypto_builtin("ed25519_verify", &[public_key, s(&huge_message), sig]).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn jwt_verify_rejects_oversized_token() {
        let huge_token = "x".repeat(MAX_JWT_TOKEN_LEN + 1);
        let result = crypto_builtin("jwt_verify", &[s(&huge_token), long_secret()]).unwrap();
        let obj = match result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["valid"], Value::Bool(false));
        assert_eq!(obj["payload"], Value::Null);
        assert_eq!(obj["header"], Value::Null);
        assert_eq!(obj["reason"], Value::Str("input_too_large".to_string()));
    }

    #[test]
    fn jwt_sign_rejects_short_secret() {
        let mut payload = HashMap::new();
        payload.insert("sub".to_string(), s("user-1"));
        let err = crypto_builtin("jwt_sign", &[Value::Object(payload), s("too-short")]);
        assert!(err.is_err());
    }

    #[test]
    fn jwt_verify_rejects_short_secret_without_throwing() {
        // Forge a syntactically valid HS256 token by signing with a long
        // secret, then verify it with a short one — jwt_verify must report
        // this as a structured invalid result, never throw.
        let mut payload = HashMap::new();
        payload.insert("exp".to_string(), Value::Number((now_secs() + 3600) as f64));
        let token = crypto_builtin("jwt_sign", &[Value::Object(payload), long_secret()]).unwrap();
        let result = crypto_builtin("jwt_verify", &[token, s("short")]).unwrap();
        let obj = match result {
            Value::Object(m) => m,
            _ => panic!("expected object"),
        };
        assert_eq!(obj["valid"], Value::Bool(false));
        let reason = obj["reason"].as_str().unwrap();
        assert!(reason.contains("32 bytes"));
    }

    // ── Hex encoder correctness ───────────────────────────────────────────────

    #[test]
    fn hex_encode_matches_known_values() {
        assert_eq!(hex_encode(&[]), "");
        assert_eq!(hex_encode(&[0x00]), "00");
        assert_eq!(hex_encode(&[0xff]), "ff");
        assert_eq!(hex_encode(&[0x0a, 0xb1, 0xc2]), "0ab1c2");
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    #[test]
    fn hex_encode_roundtrips_through_hex_decode() {
        let bytes: Vec<u8> = (0..=255).collect();
        let encoded = hex_encode(&bytes);
        assert_eq!(encoded.len(), 512);
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, bytes);
    }
}
