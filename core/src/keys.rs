//! The key exchange over `device`'s contract (`docs/01-deappje/02-device.md`):
//! the first key of a group, wrapped for the device that minted it; a key
//! request from a device that holds nothing, verified by the approver
//! through the fingerprint on its screen or the tag of the invite it came
//! through; the fulfilment, one wrap per key id the approver holds; the
//! rotation after a removal, one publisher per key id, the loser adopting;
//! and receiving: every wrap for this device opened under its pinned
//! sender, refused and surfaced when a known device offers another key.

use std::fmt;

use uuid::Uuid;

use crate::api::{ApiError, Code};
use crate::crypto::CryptoError;
use crate::invite::InviteSecret;
use crate::keyring::{GroupKey, Keyring, WrapIds, unwrap, wrap};
use crate::rng::CoreRng;
use crate::shell::DeviceKey;
use crate::trust::{PinMismatch, Pins, fingerprint};

/// A device as another member sees it: what wrapping for it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The device.
    pub id: Uuid,
    /// Its user.
    pub user_id: Uuid,
    /// Its X25519 public key, 32 bytes.
    pub public_key: Vec<u8>,
    /// `ios`, `android` or `web`.
    pub platform: String,
}

/// An open key request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRequest {
    /// The request.
    pub id: Uuid,
    /// The group.
    pub group_id: Uuid,
    /// The device asking.
    pub device: DeviceInfo,
    /// The invite it came through, if one.
    pub invite_id: Option<Uuid>,
    /// `HMAC-SHA256(invite secret, device public key)`, when it came
    /// through an invite.
    pub invite_tag: Vec<u8>,
    /// When it was made.
    pub created_at_ms: i64,
}

/// One copy of a group key, wrapped for one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKey {
    /// The row.
    pub id: Uuid,
    /// The group.
    pub group_id: Uuid,
    /// The key generation.
    pub key_id: u32,
    /// The device it is wrapped for.
    pub device_id: Uuid,
    /// The HPKE output.
    pub wrapped: Vec<u8>,
    /// The device that wrapped it.
    pub wrapped_by_device: Uuid,
    /// That device's public key; empty when it is gone.
    pub wrapped_by_public_key: Vec<u8>,
}

/// One wrap of a published key, for one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapFor {
    /// The device.
    pub device_id: Uuid,
    /// The HPKE output.
    pub wrapped: Vec<u8>,
}

/// `device` as the key exchange calls it. Every call names the calling
/// device; the caller's user is the session's.
#[allow(async_fn_in_trait)] // the core's own seam
pub trait Devices {
    /// The calling device, by the id it minted; a repeat refreshes.
    async fn register_device(&self, device_id: Uuid, public_key: [u8; 32], platform: &str) -> Result<(), ApiError>;
    /// The devices holding a key for the group; the caller must hold one.
    async fn list_devices(&self, device_id: Uuid, group_id: Uuid) -> Result<Vec<DeviceInfo>, ApiError>;
    /// Asks the holders for the group's keys.
    async fn request_group_keys(&self, device_id: Uuid, group_id: Uuid, invite_id: Option<Uuid>, invite_tag: Option<[u8; 32]>) -> Result<KeyRequest, ApiError>;
    /// The open requests in the groups the caller holds a key for.
    async fn list_key_requests(&self, device_id: Uuid) -> Result<Vec<KeyRequest>, ApiError>;
    /// Hands the requesting device the keys, one wrap per key id.
    async fn fulfil_key_request(&self, device_id: Uuid, request_id: Uuid, keys: Vec<(u32, Vec<u8>)>) -> Result<(), ApiError>;
    /// Publishes one key wrapped for each listed device; answers what
    /// landed.
    async fn publish_wrapped_keys(&self, device_id: Uuid, group_id: Uuid, key_id: u32, wraps: Vec<WrapFor>) -> Result<Vec<WrappedKey>, ApiError>;
    /// Every key wrapped for the calling device, or those of one group.
    async fn get_wrapped_keys(&self, device_id: Uuid, group_id: Option<Uuid>) -> Result<Vec<WrappedKey>, ApiError>;
}

/// What the exchange did, for the shell to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyEvent {
    /// A key was received and is in the keyring.
    Received {
        /// The group.
        group_id: Uuid,
        /// The generation.
        key_id: u32,
    },
    /// A wrap could not be opened: its wrapping device is gone, its bytes
    /// are not a wrap, or it was not made for this device. The device asks
    /// for the keys again.
    Unopenable {
        /// The group.
        group_id: Uuid,
        /// The generation.
        key_id: u32,
    },
    /// A known device offered another public key: refused, and the
    /// person decides.
    Pin(PinMismatch),
    /// A key this device minted was published.
    Published {
        /// The group.
        group_id: Uuid,
        /// The generation.
        key_id: u32,
        /// The devices the wrap landed for.
        devices: Vec<Uuid>,
    },
    /// Another device published the generation first; this device adopted
    /// it.
    Adopted {
        /// The group.
        group_id: Uuid,
        /// The generation.
        key_id: u32,
    },
}

/// Why the exchange stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The server refused or could not be reached.
    Api(ApiError),
    /// A known device offers another key.
    Pin(PinMismatch),
    /// This device holds no key of the group.
    NoKey {
        /// The group.
        group_id: Uuid,
    },
    /// A public key handed over is not 32 bytes.
    PublicKey {
        /// The device.
        device_id: Uuid,
    },
    /// A wrap could not be made.
    Crypto(CryptoError),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Api(e) => write!(f, "{e}"),
            KeyError::Pin(e) => write!(f, "{e}"),
            KeyError::NoKey { group_id } => write!(f, "no key of group {group_id}"),
            KeyError::PublicKey { device_id } => write!(f, "device {device_id}: public key is not 32 bytes"),
            KeyError::Crypto(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<ApiError> for KeyError {
    fn from(e: ApiError) -> Self {
        KeyError::Api(e)
    }
}

/// A key request as the approver sees it: the fingerprint to compare with
/// the requesting device's screen, and whether its invite tag checks out
/// under an invite secret this device holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// The request.
    pub request: KeyRequest,
    /// The requesting device's fingerprint, `xxxx-xxxx-xxxx`.
    pub fingerprint: String,
    /// `Some(true)` when the tag is the invite's over this key, `Some(false)`
    /// when it is not, `None` when the request names no invite or the
    /// device holds no secret for it.
    pub invite_verified: Option<bool>,
}

fn pk32(bytes: &[u8], device_id: Uuid) -> Result<[u8; 32], KeyError> {
    bytes.try_into().map_err(|_| KeyError::PublicKey { device_id })
}

/// The exchange, over this device's key, keyring and pins.
pub struct KeyExchange<'a> {
    /// This device.
    pub device_id: Uuid,
    /// Its key, behind the shell's seam.
    pub device_key: &'a dyn DeviceKey,
    /// The group keys.
    pub keyring: &'a mut Keyring,
    /// The public keys pinned per device.
    pub pins: &'a mut Pins,
    /// The generator.
    pub rng: &'a mut CoreRng,
}

impl KeyExchange<'_> {
    fn my_public_key(&self) -> [u8; 32] {
        self.device_key.public_key()
    }

    /// Registers this device (its id, its public key, the platform).
    pub async fn register<D: Devices>(&mut self, devices: &D, platform: &str) -> Result<(), KeyError> {
        Ok(devices.register_device(self.device_id, self.my_public_key(), platform).await?)
    }

    /// Wraps `key` for a device, checking its public key against the pin.
    fn wrap_for(&mut self, key: &GroupKey, group_id: Uuid, key_id: u32, device_id: Uuid, public_key: &[u8]) -> Result<Vec<u8>, KeyError> {
        let pk = pk32(public_key, device_id)?;
        self.pins.check(device_id, pk).map_err(KeyError::Pin)?;
        let ids = WrapIds { group_id, key_id, device_id, wrapped_by_device: self.device_id };
        wrap(key, self.device_key, &pk, ids, self.rng).map_err(KeyError::Crypto)
    }

    /// A group's first key: minted here, wrapped for this device, published
    /// as generation 1, held.
    pub async fn create_group_key<D: Devices>(&mut self, devices: &D, group_id: Uuid) -> Result<Vec<KeyEvent>, KeyError> {
        let key = GroupKey::mint(self.rng);
        let me = self.device_id;
        let wrapped = self.wrap_for(&key, group_id, 1, me, &self.my_public_key())?;
        let landed = devices.publish_wrapped_keys(me, group_id, 1, vec![WrapFor { device_id: me, wrapped }]).await?;
        self.keyring.insert(group_id, 1, key);
        Ok(vec![KeyEvent::Published { group_id, key_id: 1, devices: landed.iter().map(|k| k.device_id).collect() }])
    }

    /// Asks the group's holders for its keys, with the tag of the invite
    /// the device came through when it did.
    pub async fn request_keys<D: Devices>(&mut self, devices: &D, group_id: Uuid, invite: Option<(Uuid, &InviteSecret)>) -> Result<KeyRequest, KeyError> {
        let (invite_id, tag) = match invite {
            Some((id, secret)) => (Some(id), Some(secret.tag(&self.my_public_key()))),
            None => (None, None),
        };
        Ok(devices.request_group_keys(self.device_id, group_id, invite_id, tag).await?)
    }

    /// The open requests this device may answer, each with what the person
    /// needs to decide; `secret_of` finds the invite secret the device
    /// holds for an invite (the group's `invite_secret` objects).
    pub async fn pending_requests<D: Devices>(&self, devices: &D, secret_of: impl Fn(Uuid) -> Option<InviteSecret>) -> Result<Vec<PendingRequest>, KeyError> {
        let requests = devices.list_key_requests(self.device_id).await?;
        Ok(requests
            .into_iter()
            .filter(|r| r.device.id != self.device_id)
            .map(|request| {
                let pk: Option<[u8; 32]> = request.device.public_key.as_slice().try_into().ok();
                let fingerprint = pk.map(|pk| fingerprint(&pk)).unwrap_or_default();
                let invite_verified = match (request.invite_id, pk) {
                    (Some(invite_id), Some(pk)) => secret_of(invite_id).map(|s| s.verify(&pk, &request.invite_tag)),
                    _ => None,
                };
                PendingRequest { request, fingerprint, invite_verified }
            })
            .collect())
    }

    /// Hands the requesting device every key this device holds for the
    /// group, pinning the requester's key: the person approved it, by the
    /// fingerprint or the invite.
    pub async fn approve<D: Devices>(&mut self, devices: &D, request: &KeyRequest) -> Result<(), KeyError> {
        let group_id = request.group_id;
        let key_ids = self.keyring.key_ids(group_id);
        if key_ids.is_empty() {
            return Err(KeyError::NoKey { group_id });
        }
        let mut keys = Vec::with_capacity(key_ids.len());
        for key_id in key_ids {
            let key = self.keyring.get(group_id, key_id).expect("listed").clone();
            keys.push((key_id, self.wrap_for(&key, group_id, key_id, request.device.id, &request.device.public_key)?));
        }
        Ok(devices.fulfil_key_request(self.device_id, request.id, keys).await?)
    }

    /// Takes every wrap for this device (of one group, or all) into the
    /// keyring: opened under the wrapping device's key, which is pinned.
    pub async fn receive<D: Devices>(&mut self, devices: &D, group_id: Option<Uuid>) -> Result<Vec<KeyEvent>, KeyError> {
        let mut events = Vec::new();
        for k in devices.get_wrapped_keys(self.device_id, group_id).await? {
            if self.keyring.get(k.group_id, k.key_id).is_some() {
                continue;
            }
            let Ok(sender_pk) = pk32(&k.wrapped_by_public_key, k.wrapped_by_device) else {
                events.push(KeyEvent::Unopenable { group_id: k.group_id, key_id: k.key_id });
                continue;
            };
            if let Err(mismatch) = self.pins.check(k.wrapped_by_device, sender_pk) {
                events.push(KeyEvent::Pin(mismatch));
                continue;
            }
            let ids = WrapIds { group_id: k.group_id, key_id: k.key_id, device_id: self.device_id, wrapped_by_device: k.wrapped_by_device };
            match unwrap(&k.wrapped, self.device_key, &sender_pk, ids) {
                Ok(key) => {
                    self.keyring.insert(k.group_id, k.key_id, key);
                    events.push(KeyEvent::Received { group_id: k.group_id, key_id: k.key_id });
                }
                Err(_) => events.push(KeyEvent::Unopenable { group_id: k.group_id, key_id: k.key_id }),
            }
        }
        Ok(events)
    }

    /// After a removal: mints the next generation and wraps it for every
    /// holder of the current one, this device included. Another device
    /// publishing the generation first is adopted instead.
    pub async fn rotate<D: Devices>(&mut self, devices: &D, group_id: Uuid) -> Result<Vec<KeyEvent>, KeyError> {
        let Some((current, _)) = self.keyring.latest(group_id) else { return Err(KeyError::NoKey { group_id }) };
        let next = current + 1;
        let holders = devices.list_devices(self.device_id, group_id).await?;
        let key = GroupKey::mint(self.rng);
        let mut wraps = Vec::with_capacity(holders.len());
        for d in &holders {
            wraps.push(WrapFor { device_id: d.id, wrapped: self.wrap_for(&key, group_id, next, d.id, &d.public_key)? });
        }
        match devices.publish_wrapped_keys(self.device_id, group_id, next, wraps).await {
            Ok(landed) => {
                self.keyring.insert(group_id, next, key);
                Ok(vec![KeyEvent::Published { group_id, key_id: next, devices: landed.iter().map(|k| k.device_id).collect() }])
            }
            Err(e) if e.code == Code::FailedPrecondition => {
                let mut events = self.receive(devices, Some(group_id)).await?;
                if self.keyring.get(group_id, next).is_some() {
                    events.push(KeyEvent::Adopted { group_id, key_id: next });
                }
                Ok(events)
            }
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{DeviceWakeup, DevicesModel, Role, block_on};
    use crate::shell::SoftwareDeviceKey;

    struct Dev {
        id: Uuid,
        user: Uuid,
        key: SoftwareDeviceKey,
        keyring: Keyring,
        pins: Pins,
        rng: CoreRng,
    }

    impl Dev {
        fn new(n: u8, user: u128, model: &DevicesModel) -> Dev {
            let mut rng = CoreRng::from_seed([n; 32]);
            let id = Uuid::from_u128(0xd000 + u128::from(n));
            let user = Uuid::from_u128(user);
            model.session(id, user);
            let key = SoftwareDeviceKey::generate(&mut rng);
            let mut dev = Dev { id, user, key, keyring: Keyring::new(), pins: Pins::new(), rng };
            let platform = if n.is_multiple_of(2) { "ios" } else { "android" };
            block_on(dev.kx().register(model, platform)).unwrap();
            dev
        }

        fn kx(&mut self) -> KeyExchange<'_> {
            KeyExchange { device_id: self.id, device_key: &self.key, keyring: &mut self.keyring, pins: &mut self.pins, rng: &mut self.rng }
        }

        fn receive(&mut self, model: &DevicesModel, group: Uuid) -> Vec<KeyEvent> {
            block_on(self.kx().receive(model, Some(group))).unwrap()
        }
    }

    fn group() -> Uuid {
        Uuid::from_u128(0x6)
    }

    #[test]
    fn the_first_key_is_minted_wrapped_for_its_device_and_received() {
        let m = DevicesModel::new([1; 32]);
        let g = group();
        let mut a1 = Dev::new(1, 1, &m);
        m.add_member(g, a1.user, Role::Admin);
        let events = block_on(a1.kx().create_group_key(&m, g)).unwrap();
        assert_eq!(events, vec![KeyEvent::Published { group_id: g, key_id: 1, devices: vec![a1.id] }]);
        assert_eq!(a1.keyring.key_ids(g), vec![1]);
        assert_eq!(m.holders(g), vec![a1.id]);
        // The same device on a fresh keyring (a reinstall that kept its
        // device key) opens its own wrap.
        let held = a1.keyring.get(g, 1).unwrap().clone();
        a1.keyring = Keyring::new();
        assert_eq!(a1.receive(&m, g), vec![KeyEvent::Received { group_id: g, key_id: 1 }]);
        assert_eq!(a1.keyring.get(g, 1), Some(&held));
        assert!(a1.receive(&m, g).is_empty(), "nothing new");
    }

    #[test]
    fn a_request_through_an_invite_is_verified_approved_and_the_keys_land() {
        let m = DevicesModel::new([2; 32]);
        let g = group();
        let mut a1 = Dev::new(1, 1, &m);
        let mut b1 = Dev::new(2, 2, &m);
        m.add_member(g, a1.user, Role::Admin);
        m.add_member(g, b1.user, Role::Member);
        block_on(a1.kx().create_group_key(&m, g)).unwrap();
        let invite_id = Uuid::from_u128(0x1);
        let secret = InviteSecret::mint(&mut a1.rng);
        let request = block_on(b1.kx().request_keys(&m, g, Some((invite_id, &secret)))).unwrap();
        assert_eq!(request.device.id, b1.id);
        assert_eq!(m.take_wakeups(a1.id), vec![DeviceWakeup::KeyRequest { group_id: g, request_id: request.id }]);

        let right = secret.clone();
        let pending = block_on(a1.kx().pending_requests(&m, |id| (id == invite_id).then(|| right.clone()))).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].fingerprint, fingerprint(&b1.key.public_key()));
        assert_eq!(pending[0].invite_verified, Some(true));
        let wrong = InviteSecret::mint(&mut a1.rng);
        let pending_wrong = block_on(a1.kx().pending_requests(&m, |_| Some(wrong.clone()))).unwrap();
        assert_eq!(pending_wrong[0].invite_verified, Some(false), "a tag under another secret");
        let pending_none = block_on(a1.kx().pending_requests(&m, |_| None)).unwrap();
        assert_eq!(pending_none[0].invite_verified, None, "no secret held: the fingerprint decides");
        assert!(block_on(b1.kx().pending_requests(&m, |_| None)).unwrap().is_empty(), "a non-holder sees none");

        block_on(a1.kx().approve(&m, &pending[0].request)).unwrap();
        assert_eq!(m.open_requests(g), 0);
        assert_eq!(m.take_wakeups(b1.id), vec![DeviceWakeup::Keys { group_id: g }]);
        assert_eq!(b1.receive(&m, g), vec![KeyEvent::Received { group_id: g, key_id: 1 }]);
        assert_eq!(b1.keyring.get(g, 1), a1.keyring.get(g, 1), "the same key");
        assert_eq!(a1.pins.get(b1.id), Some(&b1.key.public_key()), "the approver pinned the requester");
        assert_eq!(b1.pins.get(a1.id), Some(&a1.key.public_key()), "the requester pinned the approver");
        assert_eq!(block_on(b1.kx().request_keys(&m, g, None)).unwrap_err(), KeyError::Api(ApiError::new(Code::FailedPrecondition, "the device already holds a key for this group")));
    }

    #[test]
    fn a_viewer_approves_only_its_own_device() {
        let m = DevicesModel::new([3; 32]);
        let g = group();
        let mut a1 = Dev::new(1, 1, &m);
        let mut c1 = Dev::new(3, 3, &m);
        let mut c2 = Dev::new(4, 3, &m);
        let mut b1 = Dev::new(2, 2, &m);
        m.add_member(g, a1.user, Role::Admin);
        m.add_member(g, c1.user, Role::Viewer);
        m.add_member(g, b1.user, Role::Member);
        block_on(a1.kx().create_group_key(&m, g)).unwrap();
        let c1_req = block_on(c1.kx().request_keys(&m, g, None)).unwrap();
        block_on(a1.kx().approve(&m, &c1_req)).unwrap();
        c1.receive(&m, g);
        let b1_req = block_on(b1.kx().request_keys(&m, g, None)).unwrap();
        assert!(matches!(block_on(c1.kx().approve(&m, &b1_req)), Err(KeyError::Api(ApiError { code: Code::PermissionDenied, .. }))));
        let c2_req = block_on(c2.kx().request_keys(&m, g, None)).unwrap();
        block_on(c1.kx().approve(&m, &c2_req)).unwrap();
        assert_eq!(c2.receive(&m, g), vec![KeyEvent::Received { group_id: g, key_id: 1 }]);
    }

    #[test]
    fn a_rotation_reaches_every_holder_and_the_loser_adopts() {
        let m = DevicesModel::new([4; 32]);
        let g = group();
        let mut a1 = Dev::new(1, 1, &m);
        let mut b1 = Dev::new(2, 2, &m);
        let mut c1 = Dev::new(3, 3, &m);
        for d in [&a1, &b1, &c1] {
            m.add_member(g, d.user, Role::Member);
        }
        block_on(a1.kx().create_group_key(&m, g)).unwrap();
        for d in [&mut b1, &mut c1] {
            let req = block_on(d.kx().request_keys(&m, g, None)).unwrap();
            block_on(a1.kx().approve(&m, &req)).unwrap();
            d.receive(&m, g);
        }
        m.remove_member(g, c1.user);
        assert!(m.take_wakeups(b1.id).contains(&DeviceWakeup::Rotate { group_id: g }), "every remaining holder is told to rotate");
        let events = block_on(a1.kx().rotate(&m, g)).unwrap();
        assert!(matches!(&events[..], [KeyEvent::Published { key_id: 2, devices, .. }] if devices.len() == 2), "{events:?}");
        assert_eq!(b1.receive(&m, g), vec![KeyEvent::Received { group_id: g, key_id: 2 }]);
        assert_eq!(b1.keyring.get(g, 2), a1.keyring.get(g, 2));
        assert!(c1.receive(&m, g).is_empty(), "the removed member's device gets nothing");
        assert_eq!(c1.keyring.key_ids(g), vec![1], "and keeps only what it had");

        // Both rotate at once: the second publisher of key 3 adopts.
        block_on(a1.kx().rotate(&m, g)).unwrap();
        let events = block_on(b1.kx().rotate(&m, g)).unwrap();
        assert!(events.contains(&KeyEvent::Adopted { group_id: g, key_id: 3 }), "{events:?}");
        assert_eq!(b1.keyring.get(g, 3), a1.keyring.get(g, 3));
        assert_eq!(a1.keyring.key_ids(g), vec![1, 2, 3], "every generation stays");
    }

    #[test]
    fn a_changed_key_under_a_known_device_is_refused_and_a_gone_wrapper_is_unopenable() {
        let m = DevicesModel::new([5; 32]);
        let g = group();
        let mut a1 = Dev::new(1, 1, &m);
        let mut b1 = Dev::new(2, 2, &m);
        m.add_member(g, a1.user, Role::Admin);
        m.add_member(g, b1.user, Role::Member);
        block_on(a1.kx().create_group_key(&m, g)).unwrap();
        let req = block_on(b1.kx().request_keys(&m, g, None)).unwrap();
        block_on(a1.kx().approve(&m, &req)).unwrap();
        b1.receive(&m, g);
        block_on(a1.kx().rotate(&m, g)).unwrap();
        // A hostile server reports another key for a1 before b1 opens the
        // rotation: refused, surfaced, nothing held.
        m.swap_public_key(a1.id, [7; 32]);
        let events = b1.receive(&m, g);
        assert!(matches!(&events[..], [KeyEvent::Pin(PinMismatch { device_id, .. })] if *device_id == a1.id), "{events:?}");
        assert_eq!(b1.keyring.key_ids(g), vec![1]);
        m.swap_public_key(a1.id, a1.key.public_key());
        assert_eq!(b1.receive(&m, g), vec![KeyEvent::Received { group_id: g, key_id: 2 }]);
        // a1 goes: a later wrap of its cannot be opened by a fresh keyring.
        m.remove_device(a1.id);
        b1.keyring = Keyring::new();
        let events = b1.receive(&m, g);
        assert_eq!(events, vec![KeyEvent::Unopenable { group_id: g, key_id: 1 }, KeyEvent::Unopenable { group_id: g, key_id: 2 }]);
    }
}
