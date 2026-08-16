//! Cookie and credential encryption at rest ([[Session Manager]] §4.8).
//!
//! Redis holds only ciphertext. An identity's cookie jar and credentials are sealed with
//! XChaCha20-Poly1305 under a key from the secrets file, with a fresh random 24-byte nonce per seal
//! (XChaCha's nonce is large enough that random nonces do not collide in practice, so no counter to
//! keep). Plaintext exists only for the duration of a lease and is zeroised on drop — never written
//! to a log, a metric, a trace, or a DLQ payload.
//!
//! The nonce is prepended to the ciphertext, so a sealed blob is self-contained: `nonce ‖ ct`. That
//! is what lets a caller store one opaque string per identity and hand it back to `open` without
//! tracking nonces separately.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

/// XChaCha20-Poly1305 nonce length.
const NONCE_LEN: usize = 24;
/// Key length — 32 bytes.
pub const KEY_LEN: usize = 32;

/// Why a seal or open failed.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("key must be {KEY_LEN} bytes, got {0}")]
    KeyLength(usize),
    #[error("sealed blob is too short to contain a nonce")]
    Truncated,
    /// Wrong key, corrupted ciphertext, or a tampered tag — deliberately indistinguishable, so a
    /// caller cannot use the error to probe which.
    #[error("decryption failed")]
    OpenFailed,
}

/// Seals and opens an identity's secrets under one key.
#[derive(Clone)]
pub struct CookieCrypto {
    cipher: XChaCha20Poly1305,
}

impl CookieCrypto {
    /// Build from a 32-byte key read from the secrets file.
    pub fn new(key: &[u8]) -> Result<Self, CryptoError> {
        if key.len() != KEY_LEN {
            return Err(CryptoError::KeyLength(key.len()));
        }
        Ok(Self {
            cipher: XChaCha20Poly1305::new(key.into()),
        })
    }

    /// Seal `plaintext`, returning `nonce ‖ ciphertext`. A fresh random nonce each call, so sealing
    /// the same cookies twice yields different blobs and reveals nothing by comparison.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| CryptoError::OpenFailed)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(nonce.as_slice());
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Open a `nonce ‖ ciphertext` blob. The returned plaintext is wrapped in [`Zeroizing`], so it
    /// is wiped from memory when the caller drops it (§4.8).
    pub fn open(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        if blob.len() < NONCE_LEN {
            return Err(CryptoError::Truncated);
        }
        let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
        let nonce = XNonce::from_slice(nonce_bytes);
        let pt = self
            .cipher
            .decrypt(nonce, ct)
            .map_err(|_| CryptoError::OpenFailed)?;
        Ok(Zeroizing::new(pt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; KEY_LEN] {
        // A fixed test key. Real keys come from the secrets file.
        let mut k = [0u8; KEY_LEN];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    #[test]
    fn a_sealed_cookie_jar_round_trips() {
        let c = CookieCrypto::new(&key()).unwrap();
        let secret = b"sessionid=abc123; csrftoken=deadbeef";
        let blob = c.seal(secret).unwrap();
        assert_ne!(
            &blob[..],
            &secret[..],
            "the store holds ciphertext, not plaintext"
        );
        assert_eq!(&c.open(&blob).unwrap()[..], secret);
    }

    #[test]
    fn the_same_plaintext_seals_differently_each_time() {
        let c = CookieCrypto::new(&key()).unwrap();
        let a = c.seal(b"same").unwrap();
        let b = c.seal(b"same").unwrap();
        assert_ne!(a, b, "a fresh nonce per seal must randomise the blob");
        // Both still open to the same plaintext.
        assert_eq!(&c.open(&a).unwrap()[..], b"same");
        assert_eq!(&c.open(&b).unwrap()[..], b"same");
    }

    #[test]
    fn a_wrong_key_cannot_open_the_blob() {
        let c = CookieCrypto::new(&key()).unwrap();
        let blob = c.seal(b"secret").unwrap();
        let mut other = key();
        other[0] ^= 0xff;
        let wrong = CookieCrypto::new(&other).unwrap();
        assert!(matches!(wrong.open(&blob), Err(CryptoError::OpenFailed)));
    }

    #[test]
    fn a_tampered_blob_is_rejected() {
        let c = CookieCrypto::new(&key()).unwrap();
        let mut blob = c.seal(b"secret").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01; // flip a ciphertext/tag bit
        assert!(matches!(c.open(&blob), Err(CryptoError::OpenFailed)));
    }

    #[test]
    fn a_wrong_length_key_and_a_truncated_blob_are_refused() {
        assert!(matches!(
            CookieCrypto::new(&[0u8; 16]),
            Err(CryptoError::KeyLength(16))
        ));
        let c = CookieCrypto::new(&key()).unwrap();
        assert!(matches!(c.open(b"short"), Err(CryptoError::Truncated)));
    }
}
