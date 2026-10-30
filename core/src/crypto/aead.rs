//! The two AEADs: XChaCha20-Poly1305 with a random 24-byte nonce for object
//! values and blobs (`docs/01-deappje/02-device.md`), and ChaCha20-Poly1305
//! with a 12-byte nonce, which is what HPKE's suite uses.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, XChaCha20Poly1305};

use super::CryptoError;

/// XChaCha20-Poly1305 seal.
pub fn xchacha_seal(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(nonce.into(), Payload { msg: plaintext, aad })
        .expect("XChaCha20-Poly1305 seals any message that fits in memory")
}

/// XChaCha20-Poly1305 open.
pub fn xchacha_open(key: &[u8; 32], nonce: &[u8; 24], aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(nonce.into(), Payload { msg: ciphertext, aad })
        .map_err(|_| CryptoError::Authentication)
}

/// ChaCha20-Poly1305 seal (HPKE's AEAD).
pub fn chacha_seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(key.into())
        .encrypt(nonce.into(), Payload { msg: plaintext, aad })
        .expect("ChaCha20-Poly1305 seals any message that fits in memory")
}

/// ChaCha20-Poly1305 open.
pub fn chacha_open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    ChaCha20Poly1305::new(key.into())
        .decrypt(nonce.into(), Payload { msg: ciphertext, aad })
        .map_err(|_| CryptoError::Authentication)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_refuses_a_changed_aad_or_byte() {
        let key = [1u8; 32];
        let nonce = [2u8; 24];
        let ct = xchacha_seal(&key, &nonce, b"aad", b"hello");
        assert_eq!(ct.len(), 5 + 16);
        assert_eq!(xchacha_open(&key, &nonce, b"aad", &ct).unwrap(), b"hello");
        assert_eq!(xchacha_open(&key, &nonce, b"aaD", &ct), Err(CryptoError::Authentication));
        let mut bad = ct.clone();
        bad[0] ^= 1;
        assert_eq!(xchacha_open(&key, &nonce, b"aad", &bad), Err(CryptoError::Authentication));
        assert_eq!(xchacha_open(&[3; 32], &nonce, b"aad", &ct), Err(CryptoError::Authentication));
        let short = chacha_seal(&key, &[0; 12], b"", b"");
        assert_eq!(chacha_open(&key, &[0; 12], b"", &short).unwrap(), b"");
    }
}
