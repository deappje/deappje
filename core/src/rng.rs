//! The core's random generator: ChaCha20, seeded from the shell's entropy.
//! Everything random in the core (nonces, group keys, ephemeral HPKE keys,
//! object ids, invite secrets) comes from here, so a test that seeds it
//! fixes every output.

use rand_core::{RngCore, SeedableRng};

use crate::shell::Random;

/// The generator. Not `Clone`: two copies would draw the same values.
pub struct CoreRng {
    inner: rand_chacha::ChaCha20Rng,
}

impl CoreRng {
    /// Seeded from the shell's entropy.
    pub fn from_shell(random: &dyn Random) -> CoreRng {
        let mut seed = [0u8; 32];
        random.fill(&mut seed);
        CoreRng::from_seed(seed)
    }

    /// Seeded from 32 bytes: a test's determinism.
    pub fn from_seed(seed: [u8; 32]) -> CoreRng {
        CoreRng { inner: rand_chacha::ChaCha20Rng::from_seed(seed) }
    }

    /// Fills `buf`.
    pub fn fill(&mut self, buf: &mut [u8]) {
        self.inner.fill_bytes(buf);
    }

    /// 32 random bytes.
    pub fn bytes32(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        self.fill(&mut out);
        out
    }

    /// 24 random bytes: an XChaCha20-Poly1305 nonce.
    pub fn nonce24(&mut self) -> [u8; 24] {
        let mut out = [0u8; 24];
        self.fill(&mut out);
        out
    }

    /// 16 random bytes: an object id's randomness, an invite secret.
    pub fn bytes16(&mut self) -> [u8; 16] {
        let mut out = [0u8; 16];
        self.fill(&mut out);
        out
    }
}

impl RngCore for CoreRng {
    fn next_u32(&mut self) -> u32 {
        self.inner.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.inner.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.inner.fill_bytes(dest)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.inner.try_fill_bytes(dest)
    }
}

impl rand_core::CryptoRng for CoreRng {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_draws_the_same_values() {
        let mut a = CoreRng::from_seed([7; 32]);
        let mut b = CoreRng::from_seed([7; 32]);
        assert_eq!(a.bytes32(), b.bytes32());
        assert_ne!(a.bytes32(), CoreRng::from_seed([8; 32]).bytes32());
    }
}
