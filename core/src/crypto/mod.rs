//! The primitives, each behind one small function: HPKE (the key wrap),
//! the AEADs (values and blobs; HPKE's own), the hashes. Every operation
//! that needs randomness takes the core's generator; none reads the
//! device's private key, which stays behind [`crate::shell::DeviceKey`].

pub mod aead;
pub mod hash;
pub mod hpke;

/// Why a cryptographic operation refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// The ciphertext, its tag or its associated data do not match the key.
    Authentication,
    /// A Diffie-Hellman result was the all-zero point (a small-order public
    /// key), which RFC 9180 forbids using.
    NonContributory,
    /// An encapsulated key or a ciphertext of the wrong length.
    Length,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptoError::Authentication => f.write_str("authentication failed"),
            CryptoError::NonContributory => f.write_str("non-contributory Diffie-Hellman result"),
            CryptoError::Length => f.write_str("wrong length"),
        }
    }
}

impl std::error::Error for CryptoError {}
