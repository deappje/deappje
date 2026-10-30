//! What a person sees and what a device remembers about other devices'
//! keys. The fingerprint is what the approver reads off the requesting
//! device's screen; a pin is the public key a device has wrapped for or
//! received from, and a different key under a known device id is refused
//! and surfaced, which is what stops a server from swapping a holder's key
//! before a rotation (`docs/01-deappje/02-device.md`).

use std::collections::BTreeMap;
use std::fmt;

use uuid::Uuid;

use crate::crypto::hash::sha256;

/// The 32-symbol alphabet codes are read aloud in: lowercase letters and
/// digits without the pairs a reader confuses (0 and o, 1 and l). The same
/// one the server mints invite codes from.
pub const ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// The fingerprint of a device public key: the first 60 bits of its
/// SHA-256 as twelve symbols in three groups, `xxxx-xxxx-xxxx`.
pub fn fingerprint(public_key: &[u8; 32]) -> String {
    let digest = sha256(public_key);
    let mut bits: u128 = 0;
    for b in &digest[..8] {
        bits = (bits << 8) | u128::from(*b);
    }
    let mut out = String::with_capacity(14);
    for i in 0..12 {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        let symbol = (bits >> (64 - 5 * (i + 1))) & 31;
        out.push(ALPHABET[symbol as usize] as char);
    }
    out
}

/// A key offered under a device id the device knows with another key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinMismatch {
    /// The device.
    pub device_id: Uuid,
    /// The key pinned for it.
    pub pinned: [u8; 32],
    /// The key offered now.
    pub offered: [u8; 32],
}

impl fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "device {} offers key {} but {} is pinned", self.device_id, fingerprint(&self.offered), fingerprint(&self.pinned))
    }
}

impl std::error::Error for PinMismatch {}

/// The public keys pinned per device id.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Pins {
    pins: BTreeMap<Uuid, [u8; 32]>,
}

impl Pins {
    /// No pins yet.
    pub fn new() -> Pins {
        Pins::default()
    }

    /// Checks `public_key` against the pin for `device_id`, pinning it when
    /// the device is new. A different key under a known id is refused.
    pub fn check(&mut self, device_id: Uuid, public_key: [u8; 32]) -> Result<(), PinMismatch> {
        match self.pins.get(&device_id) {
            Some(pinned) if *pinned != public_key => Err(PinMismatch { device_id, pinned: *pinned, offered: public_key }),
            Some(_) => Ok(()),
            None => {
                self.pins.insert(device_id, public_key);
                Ok(())
            }
        }
    }

    /// The pinned key of a device.
    pub fn get(&self, device_id: Uuid) -> Option<&[u8; 32]> {
        self.pins.get(&device_id)
    }

    /// Replaces a pin after a person accepted the changed key (a device the
    /// person re-registered), which nothing does silently.
    pub fn accept(&mut self, device_id: Uuid, public_key: [u8; 32]) {
        self.pins.insert(device_id, public_key);
    }

    /// Forgets a device (removed, purged).
    pub fn forget(&mut self, device_id: Uuid) {
        self.pins.remove(&device_id);
    }

    /// Every pin, for the local store.
    pub fn iter(&self) -> impl Iterator<Item = (&Uuid, &[u8; 32])> {
        self.pins.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_short_stable_and_distinct() {
        let a = fingerprint(&[1; 32]);
        assert_eq!(a.len(), 14);
        assert_eq!(a, fingerprint(&[1; 32]));
        assert_ne!(a, fingerprint(&[2; 32]));
        assert!(a.bytes().enumerate().all(|(i, b)| if i == 4 || i == 9 { b == b'-' } else { ALPHABET.contains(&b) }), "{a}");
    }

    #[test]
    fn a_changed_key_under_a_known_id_is_refused_until_accepted() {
        let mut pins = Pins::new();
        let d = Uuid::from_u128(1);
        assert!(pins.check(d, [1; 32]).is_ok());
        assert!(pins.check(d, [1; 32]).is_ok(), "the same key again");
        let err = pins.check(d, [2; 32]).unwrap_err();
        assert_eq!((err.device_id, err.pinned, err.offered), (d, [1; 32], [2; 32]));
        assert_eq!(pins.get(d), Some(&[1; 32]), "the pin stands");
        pins.accept(d, [2; 32]);
        assert!(pins.check(d, [2; 32]).is_ok());
        pins.forget(d);
        assert!(pins.check(d, [3; 32]).is_ok());
    }
}
