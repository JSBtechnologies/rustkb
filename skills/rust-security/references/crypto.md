---
id: security/crypto
title: Cryptography choices
summary: >-
  Which crypto to use and how not to misuse it: never implement primitives or protocols;
  rustls with the aws-lc-rs provider for TLS; aws-lc-rs vs RustCrypto vs ring; never
  disable certificate verification; Argon2id for passwords; CSPRNG use with the rand 0.10 /
  getrandom 0.4 APIs; AEAD, MAC and JWT pitfalls.
area: security
tags: [crypto, tls, rustls, aws-lc-rs, ring, rustcrypto, argon2, password-hashing, rand, getrandom, csprng, aead, hmac, jwt, jsonwebtoken, certificates]
rust: "1.96"
edition: "2024"
crates:
  rustls: "0.23"
  aws-lc-rs: "1.18"
  ring: "0.17"
  rustls-platform-verifier: "0.7"
  rustls-pki-types: "1.15"
  reqwest: "0.13"
  argon2: "0.6"
  rand: "0.10"
  getrandom: "0.4"
  sha2: "0.11"
  hmac: "0.13"
  chacha20poly1305: "0.11"
  aes-gcm: "0.11"
  ed25519-dalek: "3.0"
  jsonwebtoken: "11.1"
  pasetors: "0.8"
  subtle: "2.6"
  webpki-roots: "1.0"
  hkdf: "0.13"
  aes-gcm-siv: "0.12"
verified: 2026-09-29
sources:
  - https://docs.rs/rustls/0.23/rustls/crypto/struct.CryptoProvider.html
  - https://docs.rs/aws-lc-rs/latest/aws_lc_rs/
  - https://github.com/RustCrypto
  - https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html
  - https://rust-random.github.io/book/guide-rngs.html
  - https://rustsec.org/advisories/RUSTSEC-2023-0071.html
---

# Cryptography choices

The rule that prevents most crypto bugs: use the highest-level, most-reviewed API that
solves the problem, with its defaults. TLS instead of a custom channel, an AEAD instead of
cipher + MAC, Argon2 instead of a hash, a CSPRNG instead of anything else.

## CRYPTO-01: Never implement cryptographic primitives or protocols

Never write your own: cipher, hash, MAC (`sha256(key || msg)` is not a MAC — length
extension), KDF, RNG, constant-time comparison, padding scheme, key exchange, "encryption"
via XOR, or a handshake protocol. Also never:

- use unauthenticated modes (AES-CBC/CTR/ECB without a MAC) — use an AEAD;
- reuse a nonce with the same key (catastrophic for AES-GCM and ChaCha20-Poly1305);
- use MD5/SHA-1 for anything security-relevant;
- hash passwords with a fast hash (SHA-2, BLAKE3) — use Argon2id (CRYPTO-05);
- invent a token format — use sessions or a standard (JWT/PASETO) via a vetted crate.

If the task seems to need a custom construction, the answer is a library that already
implements a standard (TLS, Noise, age, HPKE, OPAQUE), not new code.

## CRYPTO-02: Pick the crypto backend deliberately

| Need | Default |
|---|---|
| TLS client/server | **rustls** with its default **aws-lc-rs** provider |
| General primitives in a server/CLI (AEAD, signatures, HKDF, RSA) | **aws-lc-rs** (C/asm AWS-LC, formally verified parts, FIPS option) |
| Pure Rust: wasm, `no_std`, no C toolchain, embedded | **RustCrypto** crates (`sha2`, `hmac`, `chacha20poly1305`, `aes-gcm`, `ed25519-dalek`, `argon2`) |
| FIPS 140-3 requirement | aws-lc-rs `fips` feature + rustls `fips` feature |
| Existing ring users | `ring` 0.17 is maintained again (RUSTSEC-2025-0007 was withdrawn); fine to keep, no reason to add |
| OpenSSL | Only for FIPS/HSM/platform needs aws-lc-rs can't meet; it brings frequent C-level advisories |

RustCrypto caveat: **don't use the `rsa` crate for private-key operations** (decryption,
signing) on attacker-observable paths — RUSTSEC-2023-0071 (Marvin timing attack) has no
patched version. Use aws-lc-rs for RSA, or better, Ed25519/ECDSA P-256.

Don't mix backends without reason: two TLS stacks or two providers double the attack
surface and the advisory load (cargo-deny `bans` can enforce one — supply-chain.md).

## CRYPTO-03: rustls: choose the provider once, in main

rustls 0.23 ships with **aws-lc-rs** as its default `CryptoProvider` (needs a C compiler
at build time). The `ring` feature is the alternative. If both providers end up enabled
somewhere in the dependency graph, rustls can't choose and **panics** the first time a
config is built without an explicit provider. Binaries therefore install one at startup:

```rust
fn main() {
    // Idempotent: returns Err if a provider was already installed, which is fine.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    // ... build runtime, clients, servers
}
```

Libraries must not call `install_default`; take a `ClientConfig`/`Arc<CryptoProvider>` from
the caller or use `ClientConfig::builder_with_provider`.

Client trust roots, in order of preference:

1. **rustls-platform-verifier** — uses the OS verifier (corporate CAs, revocation where the
   OS supports it). reqwest 0.13's default `rustls` feature already does this.
2. `webpki-roots` (Mozilla's bundle compiled in) for minimal containers without a CA store.
3. A private CA added explicitly for internal services.

```rust
pub fn tls_client_config() -> Result<std::sync::Arc<rustls::ClientConfig>, rustls::Error> {
    use rustls_platform_verifier::ConfigVerifierExt;
    Ok(std::sync::Arc::new(rustls::ClientConfig::with_platform_verifier()?))
}
```

Load PEM with `rustls_pki_types::pem::PemObject` (`CertificateDer::pem_file_iter`,
`PrivateKeyDer::from_pem_file`); `rustls-pemfile` is unmaintained (RUSTSEC-2025-0134).

## CRYPTO-04: Never disable certificate verification

The single most common crypto bug in agent-written Rust:

```rust
// ❌ any network attacker can impersonate the server; never ship this, not even "for dev"
reqwest::Client::builder().danger_accept_invalid_certs(true);
// ❌ same thing, hand-rolled: a ServerCertVerifier that always returns Ok(..)
```

Fix the actual problem instead:

- Self-signed / internal CA → trust *that* CA (`reqwest::Certificate::from_pem` +
  `tls_certs_merge`, or add it to a `RootCertStore`).
- Hostname mismatch → fix the certificate SAN or the URL, not the check.
- Local development → a locally trusted dev CA (e.g. mkcert), still verified.
- Tests → a test CA generated in the test, still verified.

Also require HTTPS for anything carrying credentials and set timeouts:

```rust
pub fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
}
```

rustls only speaks TLS 1.2 and 1.3 with modern cipher suites; don't try to "harden" its
defaults. Servers: enable mTLS with `WebPkiClientVerifier` when clients are machines you
control.

## CRYPTO-05: Hash passwords with Argon2id

Default: the `argon2` crate, `Argon2::default()` (Argon2id v19, OWASP-compliant defaults),
storing the PHC string (`$argon2id$v=19$m=...`), which records salt and parameters.

```rust
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    // Generates a random salt from the OS RNG (default `getrandom` feature).
    Ok(Argon2::default().hash_password(password.as_bytes())?.to_string())
}

pub fn verify_password(password: &str, stored_phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_phc) else { return false };
    // Uses the params stored in the hash, not the Argon2 instance's.
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}
```

- The API changed in argon2 0.6 / password-hash 0.6: `hash_password(pw)` takes only the
  password and generates the salt; the old `SaltString::generate(&mut OsRng)` +
  `hash_password(pw, &salt)` pattern from tutorials no longer compiles.
- Hashing costs tens of ms of CPU and ~19 MiB of memory by default: run it in
  `tokio::task::spawn_blocking`, and rate-limit login endpoints (WEB-06) so it isn't a DoS.
- Never: SHA-256/MD5/unsalted hashes, a global salt, or reversible encryption of passwords.
  `bcrypt` is acceptable only for existing hashes (it truncates at 72 bytes); rehash to
  Argon2id on next successful login.
- Pepper (a server-side secret) via `Argon2::new_with_secret` only if you can manage and
  rotate that key.

## CRYPTO-06: Use a CSPRNG, with the current rand/getrandom API

| Purpose | Use |
|---|---|
| Session IDs, reset tokens, CSRF tokens, nonces | `rand::rng()` (ThreadRng: ChaCha12, seeded from the OS) — ≥128 bits |
| Long-term keys, seeds | `getrandom::fill` or `rand::rngs::SysRng` (OS RNG directly) |
| Simulations, games, shuffling non-secret data | `rand::rngs::SmallRng` / `StdRng::seed_from_u64` — never for secrets |

```rust
pub fn new_session_token() -> String {
    use rand::RngExt;
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn new_key() -> Result<[u8; 32], getrandom::Error> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key)?;
    Ok(key)
}
```

API drift agents trip over (rand 0.9 → 0.10, getrandom 0.2 → 0.4):

- `thread_rng()` → `rand::rng()`; `gen()`/`gen_range()` → `random()`/`random_range()`.
- In rand 0.10 the user-facing trait is **`RngExt`** (import it for `.random()`); `Rng` is
  now the rand_core generator trait (formerly `RngCore`, with `fill_bytes`/`next_u64`).
- `OsRng` → **`SysRng`**; `SeedableRng::from_os_rng` was removed.
- getrandom: `getrandom::getrandom(&mut buf)` → **`getrandom::fill(&mut buf)`**.
- `ThreadRng` is not reseeded on `fork()`; call `rand::rng().reseed()` in a forked child.

Never: seeding from time or PID, `fastrand`/`SmallRng`/`StdRng::seed_from_u64` for
secrets, 32-bit random tokens, or UUIDv1/v6/v7 as secrets (UUIDv4 has 122 random bits but
use dedicated 256-bit tokens for credentials).

## CRYPTO-07: AEAD and MAC usage

Encrypting data at rest or in an app-level envelope: an AEAD with a random 96-bit nonce
per message, nonce stored alongside the ciphertext.

```rust
use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, Generate, Key, KeyInit},
};

let key = Key::<ChaCha20Poly1305>::generate();  // or load from a KMS/secret store
let cipher = ChaCha20Poly1305::new(&key);
let nonce = Nonce::generate();                    // unique per (key, message)
let ciphertext = cipher.encrypt(&nonce, plaintext)?;
```

- Random 96-bit nonces are safe for roughly 2^32 messages per key; beyond that, rotate
  keys or use `XChaCha20Poly1305` (192-bit nonce). For nonce-misuse resistance use
  AES-GCM-SIV.
- Bind context with associated data (record ID, user ID) so ciphertexts can't be swapped
  between records.
- MACs: `Hmac<Sha256>` and verify with `mac.verify_slice(tag)` (constant time), never by
  comparing `finalize()` output with `==`.
- Derive sub-keys with HKDF instead of reusing one key for several purposes.
- Keys come from a secret manager/KMS, are held in `secrecy`/`zeroize` types
  (secure-coding.md SEC-12), and never appear in source, logs or error messages.

## CRYPTO-08: JWT: pin the algorithm and validate every claim

Default for first-party web apps: **server-side sessions** (opaque random token in an
`HttpOnly` cookie) — revocable and simpler. Use JWTs when a separate service must verify
tokens without calling the issuer.

With `jsonwebtoken` 11 (enable exactly one backend feature, `aws_lc_rs` preferred —
`rust_crypto` routes RSA through the `rsa` crate, see CRYPTO-02):

```rust
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};

#[derive(serde::Deserialize)]
pub struct Claims { pub sub: String, pub exp: u64 }

pub fn verify_jwt(token: &str, secret: &[u8]) -> jsonwebtoken::errors::Result<Claims> {
    let mut v = Validation::new(Algorithm::HS256);   // pin: never trust the header's `alg`
    v.set_audience(&["my-api"]);
    v.set_issuer(&["https://auth.example.com"]);
    v.set_required_spec_claims(&["exp", "aud", "iss", "sub"]);
    v.leeway = 30;
    Ok(decode::<Claims>(token, &DecodingKey::from_secret(secret), &v)?.claims)
}
```

- `jsonwebtoken::dangerous::insecure_decode*` skips signature checks: only for reading a
  token's header to pick a key (`kid`), never for authentication.
- HS256 secrets: ≥ 32 random bytes from a CSPRNG, not a passphrase. Prefer EdDSA/ES256
  when verifiers are other services (they then hold only the public key).
- Short `exp` (minutes) plus refresh tokens stored server-side; JWTs can't be revoked.
- Don't put secrets or PII in claims — JWTs are signed, not encrypted.
- PASETO (`pasetors`) is a less footgun-prone alternative when you control both ends.
