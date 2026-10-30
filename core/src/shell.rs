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
