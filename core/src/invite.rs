//! The invite's secret: minted on the inviting device, carried in the
//! link's fragment (`/join/<code>#<secret>`), which a browser never sends
//! and the server never sees. A joining device attaches `HMAC-SHA256(secret,
//! its public key)` to its key request; whoever holds the secret recomputes
//! the tag and so knows the request came through that link and the key is
//! the one the joining device holds (`docs/01-deappje/02-device.md`).

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::crypto::hash::{hmac_sha256, hmac_sha256_verify};
use crate::rng::CoreRng;
use crate::trust::ALPHABET;

/// The 16 random bytes of an invite link.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct InviteSecret([u8; 16]);

impl InviteSecret {
    /// A new secret from the generator, on the inviting device.
    pub fn mint(rng: &mut CoreRng) -> InviteSecret {
        InviteSecret(rng.bytes16())
    }

    /// From its bytes (the local store, the `invite_secret` object).
    pub fn from_bytes(bytes: [u8; 16]) -> InviteSecret {
        InviteSecret(bytes)
    }

    /// The bytes.
    pub fn bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// The tag a joining device attaches to its key request.
    pub fn tag(&self, device_public_key: &[u8; 32]) -> [u8; 32] {
        hmac_sha256(&self.0, device_public_key)
    }

    /// Whether `tag` is this secret's tag over the key: the holder's check
    /// before it wraps for the requesting device.
    pub fn verify(&self, device_public_key: &[u8; 32], tag: &[u8]) -> bool {
        hmac_sha256_verify(&self.0, device_public_key, tag)
    }

    /// The link's fragment: the secret in the code alphabet, 26 symbols.
    pub fn fragment(&self) -> String {
        encode32(&self.0)
    }

    /// From a link's fragment.
    pub fn from_fragment(fragment: &str) -> Option<InviteSecret> {
        let bytes = decode32(fragment, 16)?;
        Some(InviteSecret(bytes.try_into().ok()?))
    }
}

impl fmt::Debug for InviteSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InviteSecret(..)")
    }
}

/// The shared link's path and fragment: `/join/<code>#<secret>`.
pub fn join_link(code: &str, secret: &InviteSecret) -> String {
    format!("/join/{code}#{}", secret.fragment())
}

/// The code and the secret of a link, whatever origin precedes the path.
pub fn parse_join_link(link: &str) -> Option<(String, InviteSecret)> {
    let (path, fragment) = link.split_once('#')?;
    let code = path.rsplit_once("/join/")?.1;
    if code.is_empty() || code.contains('/') || code.contains('?') {
        return None;
    }
    Some((code.to_owned(), InviteSecret::from_fragment(fragment)?))
}

/// Bytes as symbols of the code alphabet, five bits each, most significant
/// first, the last symbol padded with zero bits.
fn encode32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for b in bytes {
        acc = (acc << 8) | u32::from(*b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn decode32(s: &str, len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.bytes() {
        let v = ALPHABET.iter().position(|a| *a == c)? as u32;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if out.len() != len {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_verifies_under_the_right_secret_and_key_only() {
        let mut rng = CoreRng::from_seed([7; 32]);
        let secret = InviteSecret::mint(&mut rng);
        let other = InviteSecret::mint(&mut rng);
        let pk = [1u8; 32];
        let tag = secret.tag(&pk);
        assert!(secret.verify(&pk, &tag));
        assert!(!other.verify(&pk, &tag), "the wrong secret");
        assert!(!secret.verify(&[2u8; 32], &tag), "the wrong public key");
        assert!(!secret.verify(&pk, &tag[..31]), "a truncated tag");
    }

    #[test]
    fn the_link_carries_the_secret_in_its_fragment_only() {
        let mut rng = CoreRng::from_seed([8; 32]);
        let secret = InviteSecret::mint(&mut rng);
        let link = join_link("abcdefghjk", &secret);
        assert!(link.starts_with("/join/abcdefghjk#"));
        assert_eq!(link.len(), "/join/abcdefghjk#".len() + 26);
        let (code, back) = parse_join_link(&format!("https://app.example{link}")).unwrap();
        assert_eq!(code, "abcdefghjk");
        assert_eq!(back, secret);
        assert!(parse_join_link("/join/abcdefghjk").is_none(), "no fragment");
        assert!(parse_join_link("/join/#abc").is_none(), "no code");
        assert!(parse_join_link("/join/abcdefghjk#zzz").is_none(), "a fragment of the wrong length");
        assert!(parse_join_link("/join/abcdefghjk#0000000000000000000000000O").is_none(), "symbols outside the alphabet");
    }

    #[test]
    fn the_alphabet_round_trips_bytes() {
        for bytes in [vec![0u8; 16], vec![255u8; 16], (0..16).collect::<Vec<u8>>()] {
            assert_eq!(decode32(&encode32(&bytes), 16).unwrap(), bytes);
        }
    }
}
