//! The seams a shell implements: the few things the core cannot do itself
//! on every platform the same way. A test implements them in software.

/// A source of entropy: `crypto.getRandomValues` in a browser,
/// `SecRandomCopyBytes` on iOS, `SecureRandom` on Android. The core seeds
/// its own generator from it ([`crate::rng::CoreRng`]) and asks again only
/// when it reseeds.
pub trait Random {
    /// Fills `buf` with random bytes.
    fn fill(&self, buf: &mut [u8]);
}

/// The wall clock, milliseconds since the Unix epoch, UTC. Authored and
/// edited times are stamped from it; nothing in the core waits on it.
pub trait Clock {
    /// Now.
    fn now_ms(&self) -> i64;
}

/// The device's X25519 key. The private half lives where the shell keeps
/// secrets (the Secure Enclave-backed keychain, the Android Keystore, a
/// non-extractable WebCrypto key) and never reaches the core: the core
/// delegates the Diffie-Hellman step of HPKE to it
/// (`docs/01-deappje/02-device.md`, "Key exchange protocol").
pub trait DeviceKey {
    /// The public half, 32 bytes, what `device.public_key` holds.
    fn public_key(&self) -> [u8; 32];
    /// X25519 between the private half and `their_public`. The shell
    /// returns the raw 32-byte shared point; the core checks it for the
    /// all-zero result RFC 9180 forbids.
    fn dh(&self, their_public: &[u8; 32]) -> [u8; 32];
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
    fn public_key(&self) -> [u8; 32] {
        x25519_dalek::PublicKey::from(&self.secret).to_bytes()
    }

    fn dh(&self, their_public: &[u8; 32]) -> [u8; 32] {
        self.secret.diffie_hellman(&x25519_dalek::PublicKey::from(*their_public)).to_bytes()
    }
}

/// Where the core keeps its local store: named blobs the shell persists
/// (OPFS or IndexedDB in a browser, files on a phone). The core writes
/// through on every change and reads everything back at open. The shell
/// protects it as it protects the device (the OS keeps the disk
/// encrypted); the device's private key is never in it.
pub trait Storage {
    /// The blob, if there is one.
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    /// Writes or replaces the blob.
    fn put(&self, key: &str, value: &[u8]);
    /// Removes the blob, if there is one.
    fn delete(&self, key: &str);
    /// Every key with the prefix, in order.
    fn keys(&self, prefix: &str) -> Vec<String>;
}

/// Storage in memory: tests, and a browser without "remember this
/// browser".
#[derive(Default)]
pub struct MemoryStorage {
    blobs: std::cell::RefCell<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl MemoryStorage {
    /// Empty.
    pub fn new() -> MemoryStorage {
        MemoryStorage::default()
    }

    /// How many blobs it holds.
    pub fn len(&self) -> usize {
        self.blobs.borrow().len()
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.blobs.borrow().is_empty()
    }
}

impl Storage for MemoryStorage {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.blobs.borrow().get(key).cloned()
    }

    fn put(&self, key: &str, value: &[u8]) {
        self.blobs.borrow_mut().insert(key.to_owned(), value.to_vec());
    }

    fn delete(&self, key: &str) {
        self.blobs.borrow_mut().remove(key);
    }

    fn keys(&self, prefix: &str) -> Vec<String> {
        self.blobs.borrow().range(prefix.to_owned()..).take_while(|(k, _)| k.starts_with(prefix)).map(|(k, _)| k.clone()).collect()
    }
}

/// A fixed clock for tests.
pub struct FixedClock(pub std::cell::Cell<i64>);

impl FixedClock {
    /// At `now_ms`.
    pub fn at(now_ms: i64) -> FixedClock {
        FixedClock(std::cell::Cell::new(now_ms))
    }

    /// Moves the clock forward.
    pub fn advance(&self, ms: i64) {
        self.0.set(self.0.get() + ms);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0.get()
    }
}

/// Entropy from a seed, for tests: a ChaCha20 stream.
pub struct SeededRandom(std::cell::RefCell<crate::rng::CoreRng>);

impl SeededRandom {
    /// From a seed.
    pub fn new(seed: [u8; 32]) -> SeededRandom {
        SeededRandom(std::cell::RefCell::new(crate::rng::CoreRng::from_seed(seed)))
    }
}

impl Random for SeededRandom {
    fn fill(&self, buf: &mut [u8]) {
        self.0.borrow_mut().fill(buf);
    }
}
