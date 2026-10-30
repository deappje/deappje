//! The group keys a device holds, by group and key id, and the wrap that
//! moves one to another device: HPKE Auth mode, the sender's device key
//! as the authenticator, `info` naming the group, the key id, the recipient
//! device and the wrapping device (`docs/01-deappje/02-device.md`). A
//! device's keyring holds every key id of a group, so old objects stay
//! readable after a rotation.

use std::collections::BTreeMap;

use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::crypto::{CryptoError, hpke};
use crate::rng::CoreRng;
use crate::shell::DeviceKey;

/// A group key: 32 random bytes for one `(group_id, key_id)`.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct GroupKey([u8; 32]);

impl GroupKey {
    /// A new key from the generator.
    pub fn mint(rng: &mut CoreRng) -> GroupKey {
        GroupKey(rng.bytes32())
    }

    /// From its bytes (an unwrap, a restore from the local store).
    pub fn from_bytes(bytes: [u8; 32]) -> GroupKey {
        GroupKey(bytes)
    }

    /// The bytes.
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GroupKey(..)")
    }
}

/// What a wrap is for: the key's group and id, the device it is wrapped
/// for and the device wrapping it. Both ends must agree on all four: they
/// are HPKE's `info` (group ‖ key id ‖ recipient device ‖ wrapping device,
/// each id its 16 bytes, the key id four bytes big-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrapIds {
    /// The key's group.
    pub group_id: Uuid,
    /// The key's generation.
    pub key_id: u32,
    /// The device the copy is wrapped for (`wrapped_key.device_id`).
    pub device_id: Uuid,
    /// The device that wraps (`wrapped_key.wrapped_by_device`).
    pub wrapped_by_device: Uuid,
}

impl WrapIds {
    /// The HPKE `info`.
    pub fn info(&self) -> Vec<u8> {
        let mut info = self.group_id.as_bytes().to_vec();
        info.extend_from_slice(&self.key_id.to_be_bytes());
        info.extend_from_slice(self.device_id.as_bytes());
        info.extend_from_slice(self.wrapped_by_device.as_bytes());
        info
    }
}

/// Wraps `key` for the device `ids.device_id`, whose public key is
/// `recipient_pk`, as `sender`, the device `ids.wrapped_by_device`: what
/// `PublishWrappedKeys` and `FulfilKeyRequest` carry.
pub fn wrap(key: &GroupKey, sender: &dyn DeviceKey, recipient_pk: &[u8; 32], ids: WrapIds, rng: &mut CoreRng) -> Result<Vec<u8>, CryptoError> {
    hpke::seal(sender, recipient_pk, &ids.info(), b"", key.bytes(), rng)
}

/// Unwraps what [`wrap`] produced, as the device `ids.device_id`
/// (`recipient`), verifying it came from the device `ids.wrapped_by_device`,
/// whose pinned public key is `sender_pk`.
pub fn unwrap(wrapped: &[u8], recipient: &dyn DeviceKey, sender_pk: &[u8; 32], ids: WrapIds) -> Result<GroupKey, CryptoError> {
    let bytes = hpke::open(recipient, sender_pk, &ids.info(), b"", wrapped)?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| CryptoError::Length)?;
    Ok(GroupKey(bytes))
}

/// Every group key the device holds.
#[derive(Default)]
pub struct Keyring {
    keys: BTreeMap<(Uuid, u32), GroupKey>,
}

impl Keyring {
    /// An empty keyring.
    pub fn new() -> Keyring {
        Keyring::default()
    }

    /// Holds a key; an existing one under the same ids is replaced (a
    /// re-received wrap of the same key is the same bytes).
    pub fn insert(&mut self, group_id: Uuid, key_id: u32, key: GroupKey) {
        self.keys.insert((group_id, key_id), key);
    }

    /// The key of one generation.
    pub fn get(&self, group_id: Uuid, key_id: u32) -> Option<&GroupKey> {
        self.keys.get(&(group_id, key_id))
    }

    /// The group's newest key and its id: what a new value is sealed under.
    pub fn latest(&self, group_id: Uuid) -> Option<(u32, &GroupKey)> {
        self.keys
            .range((group_id, 0)..=(group_id, u32::MAX))
            .next_back()
            .map(|((_, key_id), key)| (*key_id, key))
    }

    /// The key ids held for a group, ascending.
    pub fn key_ids(&self, group_id: Uuid) -> Vec<u32> {
        self.keys.range((group_id, 0)..=(group_id, u32::MAX)).map(|((_, k), _)| *k).collect()
    }

    /// Whether the device holds any key of the group.
    pub fn holds(&self, group_id: Uuid) -> bool {
        self.latest(group_id).is_some()
    }

    /// Forgets a group's keys (the device left it, or started over).
    pub fn forget_group(&mut self, group_id: Uuid) {
        self.keys.retain(|(g, _), _| *g != group_id);
    }

    /// The groups the device holds a key for.
    pub fn groups(&self) -> Vec<Uuid> {
        let mut out: Vec<Uuid> = self.keys.keys().map(|(g, _)| *g).collect();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{DeviceKeyExt as _, SoftwareDeviceKey};

    #[test]
    fn a_wrap_reaches_its_device_from_its_sender_only() {
        let mut rng = CoreRng::from_seed([5; 32]);
        let key = GroupKey::mint(&mut rng);
        let (g, d_a, d_b, d_c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let a = SoftwareDeviceKey::generate(&mut rng);
        let b = SoftwareDeviceKey::generate(&mut rng);
        let c = SoftwareDeviceKey::generate(&mut rng);
        let ids = WrapIds { group_id: g, key_id: 1, device_id: d_b, wrapped_by_device: d_a };
        let wrapped = wrap(&key, &a, &b.public_key(), ids, &mut rng).unwrap();
        assert_eq!(unwrap(&wrapped, &b, &a.public_key(), ids).unwrap(), key);
        assert!(unwrap(&wrapped, &c, &a.public_key(), WrapIds { device_id: d_c, ..ids }).is_err(), "another device");
        assert!(unwrap(&wrapped, &b, &c.public_key(), WrapIds { wrapped_by_device: d_c, ..ids }).is_err(), "another claimed sender");
        assert!(unwrap(&wrapped, &b, &a.public_key(), WrapIds { key_id: 2, ..ids }).is_err(), "another key id");
        assert!(unwrap(&wrapped, &b, &a.public_key(), WrapIds { group_id: Uuid::from_u128(9), ..ids }).is_err(), "another group");
        assert!(unwrap(&wrapped, &b, &a.public_key(), WrapIds { device_id: d_c, ..ids }).is_err(), "the wrap names the recipient device");
    }

    #[test]
    fn the_keyring_keeps_every_generation_and_knows_the_latest() {
        let mut rng = CoreRng::from_seed([6; 32]);
        let mut ring = Keyring::new();
        let (g, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert!(!ring.holds(g));
        let k1 = GroupKey::mint(&mut rng);
        let k2 = GroupKey::mint(&mut rng);
        ring.insert(g, 2, k2.clone());
        ring.insert(g, 1, k1.clone());
        ring.insert(h, 1, GroupKey::mint(&mut rng));
        assert_eq!(ring.latest(g), Some((2, &k2)));
        assert_eq!(ring.get(g, 1), Some(&k1));
        assert_eq!(ring.key_ids(g), vec![1, 2]);
        assert_eq!(ring.groups(), vec![g, h]);
        ring.forget_group(g);
        assert!(!ring.holds(g) && ring.holds(h));
    }
}
