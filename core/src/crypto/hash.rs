//! SHA-256 and HMAC-SHA256.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// HMAC-SHA256 of `data` under `key`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Whether `tag` is HMAC-SHA256 of `data` under `key`, compared in
/// constant time.
pub fn hmac_sha256_verify(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data);
    mac.verify_slice(tag).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_answers() {
        // SHA-256("abc"), FIPS 180-2.
        assert_eq!(
            sha256(b"abc")[..4],
            [0xba, 0x78, 0x16, 0xbf],
        );
        // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?".
        let tag = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(tag[..4], [0x5b, 0xdc, 0xc1, 0x46]);
        assert!(hmac_sha256_verify(b"Jefe", b"what do ya want for nothing?", &tag));
        assert!(!hmac_sha256_verify(b"jefe", b"what do ya want for nothing?", &tag));
    }
}
