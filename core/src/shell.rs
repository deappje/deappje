//! The seams a shell implements: the few things the core cannot do itself
//! on every platform the same way. A test implements them in software.
//! On the phones they are UniFFI foreign traits (Swift and Kotlin
//! implement them, the core holds them as `Arc<dyn ...>`), and so thread
//! safe; in the browser they are JavaScript's, on its one thread.

/// `Send + Sync` everywhere but the browser, where nothing crosses a
/// thread and JavaScript values are neither. Every seam has it as a
/// supertrait, so a trait object of one is shareable where it must be.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> MaybeSendSync for T {}
/// `Send + Sync` everywhere but the browser.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSendSync for T {}

/// A source of entropy: `crypto.getRandomValues` in a browser,
/// `SecRandomCopyBytes` on iOS, `SecureRandom` on Android. The core seeds
/// its own generator from it ([`crate::rng::CoreRng`]) and asks again only
/// when it reseeds.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(with_foreign))]
pub trait Random: MaybeSendSync {
    /// 32 random bytes.
    fn random_bytes(&self) -> Vec<u8>;
}

/// The wall clock, milliseconds since the Unix epoch, UTC. Authored and
/// edited times are stamped from it; nothing in the core waits on it.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(with_foreign))]
pub trait Clock: MaybeSendSync {
    /// Now.
    fn now_ms(&self) -> i64;
}

/// Where the session token and the device id live: the Keychain, the
/// Android Keystore, memory or OPFS in a browser. Small named strings.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(with_foreign))]
pub trait Secrets: MaybeSendSync {
    /// The value, if set.
    fn get_secret(&self, name: String) -> Option<String>;
    /// Sets or replaces the value.
    fn set_secret(&self, name: String, value: String);
    /// Removes the value.
    fn delete_secret(&self, name: String);
}

/// The device's X25519 key. The private half lives where the shell keeps
/// secrets (the Secure Enclave-backed keychain, the Android Keystore, a
/// non-extractable WebCrypto key) and never reaches the core: the core
/// delegates the Diffie-Hellman step of HPKE to it
/// (`docs/01-deappje/02-device.md`, "Key exchange protocol").
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(with_foreign))]
pub trait DeviceKey: MaybeSendSync {
    /// The public half, 32 bytes, what `device.public_key` holds.
    fn public_key_bytes(&self) -> Vec<u8>;
    /// X25519 between the private half and `their_public` (32 bytes). The
    /// shell returns the raw 32-byte shared point; the core checks it for
    /// the all-zero result RFC 9180 forbids.
    fn dh_bytes(&self, their_public: Vec<u8>) -> Vec<u8>;
}

/// The typed view of a [`DeviceKey`].
pub trait DeviceKeyExt {
    /// The public half.
    fn public_key(&self) -> [u8; 32];
    /// The Diffie-Hellman step.
    fn dh(&self, their_public: &[u8; 32]) -> [u8; 32];
}

impl<K: DeviceKey + ?Sized> DeviceKeyExt for K {
    fn public_key(&self) -> [u8; 32] {
        self.public_key_bytes().try_into().expect("a device public key is 32 bytes")
    }

    fn dh(&self, their_public: &[u8; 32]) -> [u8; 32] {
        self.dh_bytes(their_public.to_vec()).try_into().expect("an X25519 shared point is 32 bytes")
    }
}

/// A device key held in software: a test's device, and the browser without
/// "remember this browser", whose key lives in memory for the session.
pub struct SoftwareDeviceKey {
    secret: x25519_dalek::StaticSecret,
}

impl SoftwareDeviceKey {
    /// A key from 32 secret bytes (from [`Random`] through
    /// [`crate::rng::CoreRng`]).
    pub fn from_bytes(secret: [u8; 32]) -> SoftwareDeviceKey {
        SoftwareDeviceKey { secret: x25519_dalek::StaticSecret::from(secret) }
    }

    /// A fresh key from the core's generator.
    pub fn generate(rng: &mut crate::rng::CoreRng) -> SoftwareDeviceKey {
        let mut secret = [0u8; 32];
        rng.fill(&mut secret);
        SoftwareDeviceKey::from_bytes(secret)
    }
}

impl DeviceKey for SoftwareDeviceKey {
    fn public_key_bytes(&self) -> Vec<u8> {
        x25519_dalek::PublicKey::from(&self.secret).to_bytes().to_vec()
    }

    fn dh_bytes(&self, their_public: Vec<u8>) -> Vec<u8> {
        let their: [u8; 32] = their_public.try_into().expect("a peer public key is 32 bytes");
        self.secret.diffie_hellman(&x25519_dalek::PublicKey::from(their)).to_bytes().to_vec()
    }
}

/// Where the core keeps its local store: named blobs the shell persists
/// (OPFS or IndexedDB in a browser, files on a phone). The core writes
/// through on every change and reads everything back at open. The shell
/// protects it as it protects the device (the OS keeps the disk
/// encrypted); the device's private key is never in it.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(with_foreign))]
pub trait Storage: MaybeSendSync {
    /// The blob, if there is one.
    fn get(&self, key: String) -> Option<Vec<u8>>;
    /// Writes or replaces the blob.
    fn put(&self, key: String, value: Vec<u8>);
    /// Removes the blob, if there is one.
    fn delete(&self, key: String);
    /// Every key with the prefix, in order.
    fn keys(&self, prefix: String) -> Vec<String>;
}

impl<T: Storage + ?Sized> Storage for std::sync::Arc<T> {
    fn get(&self, key: String) -> Option<Vec<u8>> {
        (**self).get(key)
    }

    fn put(&self, key: String, value: Vec<u8>) {
        (**self).put(key, value)
    }

    fn delete(&self, key: String) {
        (**self).delete(key)
    }

    fn keys(&self, prefix: String) -> Vec<String> {
        (**self).keys(prefix)
    }
}

/// Storage in memory: tests, and a browser without "remember this
/// browser".
#[derive(Default)]
pub struct MemoryStorage {
    blobs: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl MemoryStorage {
    /// Empty.
    pub fn new() -> MemoryStorage {
        MemoryStorage::default()
    }

    /// How many blobs it holds.
    pub fn len(&self) -> usize {
        self.blobs.lock().expect("no poison").len()
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.blobs.lock().expect("no poison").is_empty()
    }
}

impl Storage for MemoryStorage {
    fn get(&self, key: String) -> Option<Vec<u8>> {
        self.blobs.lock().expect("no poison").get(&key).cloned()
    }

    fn put(&self, key: String, value: Vec<u8>) {
        self.blobs.lock().expect("no poison").insert(key, value);
    }

    fn delete(&self, key: String) {
        self.blobs.lock().expect("no poison").remove(&key);
    }

    fn keys(&self, prefix: String) -> Vec<String> {
        self.blobs.lock().expect("no poison").range(prefix.clone()..).take_while(|(k, _)| k.starts_with(&prefix)).map(|(k, _)| k.clone()).collect()
    }
}

/// Secrets in memory: tests, and a browser without "remember this
/// browser".
#[derive(Default)]
pub struct MemorySecrets {
    values: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

impl MemorySecrets {
    /// Empty.
    pub fn new() -> MemorySecrets {
        MemorySecrets::default()
    }
}

impl Secrets for MemorySecrets {
    fn get_secret(&self, name: String) -> Option<String> {
        self.values.lock().expect("no poison").get(&name).cloned()
    }

    fn set_secret(&self, name: String, value: String) {
        self.values.lock().expect("no poison").insert(name, value);
    }

    fn delete_secret(&self, name: String) {
        self.values.lock().expect("no poison").remove(&name);
    }
}

/// A fixed clock for tests.
pub struct FixedClock(pub std::sync::atomic::AtomicI64);

impl FixedClock {
    /// At `now_ms`.
    pub fn at(now_ms: i64) -> FixedClock {
        FixedClock(std::sync::atomic::AtomicI64::new(now_ms))
    }

    /// Moves the clock forward.
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Entropy from a seed, for tests: a ChaCha20 stream.
pub struct SeededRandom(std::sync::Mutex<crate::rng::CoreRng>);

impl SeededRandom {
    /// From a seed.
    pub fn new(seed: [u8; 32]) -> SeededRandom {
        SeededRandom(std::sync::Mutex::new(crate::rng::CoreRng::from_seed(seed)))
    }
}

impl Random for SeededRandom {
    fn random_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("no poison").bytes32().to_vec()
    }
}
