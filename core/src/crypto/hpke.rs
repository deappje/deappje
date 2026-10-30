//! HPKE, RFC 9180, in Auth mode with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256
//! and ChaCha20-Poly1305: the key wrap (`docs/01-deappje/02-device.md`).
//! Written here rather than taken from a crate because the device's static
//! private key never reaches the core: both Diffie-Hellman steps that use
//! it go through [`DeviceKey::dh`], and only the ephemeral key is the
//! core's. The tests check it against the `hpke` crate in both directions.
//!
//! Single-shot only (one message per encapsulation, sequence number 0),
//! which is all a key wrap needs.

use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use super::CryptoError;
use super::aead::{chacha_open, chacha_seal};
use crate::rng::CoreRng;
use crate::shell::DeviceKey;

const KEM_ID: u16 = 0x0020;
const KDF_ID: u16 = 0x0001;
const AEAD_ID: u16 = 0x0003;
const MODE_AUTH: u8 = 0x02;
const NK: usize = 32;
const NN: usize = 12;
const NSECRET: usize = 32;
/// The length of an encapsulated key: an X25519 public key.
pub const ENC_LEN: usize = 32;

fn kem_suite_id() -> Vec<u8> {
    let mut v = b"KEM".to_vec();
    v.extend_from_slice(&KEM_ID.to_be_bytes());
    v
}

fn hpke_suite_id() -> Vec<u8> {
    let mut v = b"HPKE".to_vec();
    v.extend_from_slice(&KEM_ID.to_be_bytes());
    v.extend_from_slice(&KDF_ID.to_be_bytes());
    v.extend_from_slice(&AEAD_ID.to_be_bytes());
    v
}

fn labeled_extract(suite_id: &[u8], salt: &[u8], label: &[u8], ikm: &[u8]) -> [u8; 32] {
    let mut labeled = b"HPKE-v1".to_vec();
    labeled.extend_from_slice(suite_id);
    labeled.extend_from_slice(label);
    labeled.extend_from_slice(ikm);
    let (prk, _) = Hkdf::<Sha256>::extract(Some(salt), &labeled);
    prk.into()
}

fn labeled_expand(suite_id: &[u8], prk: &[u8; 32], label: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let mut labeled = (len as u16).to_be_bytes().to_vec();
    labeled.extend_from_slice(b"HPKE-v1");
    labeled.extend_from_slice(suite_id);
    labeled.extend_from_slice(label);
    labeled.extend_from_slice(info);
    let mut out = vec![0u8; len];
    Hkdf::<Sha256>::from_prk(prk)
        .expect("a 32-byte PRK")
        .expand(&labeled, &mut out)
        .expect("an HKDF output of this length");
    out
}

fn contributory(dh: [u8; 32]) -> Result<[u8; 32], CryptoError> {
    if dh == [0u8; 32] { Err(CryptoError::NonContributory) } else { Ok(dh) }
}

/// The KEM's ExtractAndExpand over the two DH results, in Auth mode.
fn shared_secret(dh: &[u8], enc: &[u8; 32], pk_r: &[u8; 32], pk_s: &[u8; 32]) -> [u8; 32] {
    let suite = kem_suite_id();
    let eae_prk = labeled_extract(&suite, b"", b"eae_prk", dh);
    let mut kem_context = enc.to_vec();
    kem_context.extend_from_slice(pk_r);
    kem_context.extend_from_slice(pk_s);
    let out = labeled_expand(&suite, &eae_prk, b"shared_secret", &kem_context, NSECRET);
    out.try_into().expect("NSECRET bytes")
}

/// The key schedule: the AEAD key and the base nonce for one context.
fn key_schedule(shared_secret: &[u8; 32], info: &[u8]) -> ([u8; 32], [u8; 12]) {
    let suite = hpke_suite_id();
    let psk_id_hash = labeled_extract(&suite, b"", b"psk_id_hash", b"");
    let info_hash = labeled_extract(&suite, b"", b"info_hash", info);
    let mut context = vec![MODE_AUTH];
    context.extend_from_slice(&psk_id_hash);
    context.extend_from_slice(&info_hash);
    let secret = labeled_extract(&suite, shared_secret, b"secret", b"");
    let key = labeled_expand(&suite, &secret, b"key", &context, NK);
    let base_nonce = labeled_expand(&suite, &secret, b"base_nonce", &context, NN);
    (key.try_into().expect("NK bytes"), base_nonce.try_into().expect("NN bytes"))
}

/// Seals `plaintext` for the holder of `recipient_pk`, authenticated as
/// `sender`: the encapsulated key (`ENC_LEN` bytes) followed by the
/// ciphertext, in one buffer, which is what `wrapped_key.wrapped` holds.
pub fn seal(sender: &dyn DeviceKey, recipient_pk: &[u8; 32], info: &[u8], aad: &[u8], plaintext: &[u8], rng: &mut CoreRng) -> Result<Vec<u8>, CryptoError> {
    let sk_e = StaticSecret::from(rng.bytes32());
    let enc = PublicKey::from(&sk_e).to_bytes();
    let pk_r = PublicKey::from(*recipient_pk);
    let dh_e = sk_e.diffie_hellman(&pk_r);
    if !dh_e.was_contributory() {
        return Err(CryptoError::NonContributory);
    }
    let dh_s = contributory(sender.dh(recipient_pk))?;
    let mut dh = dh_e.to_bytes().to_vec();
    dh.extend_from_slice(&dh_s);
    let ss = shared_secret(&dh, &enc, recipient_pk, &sender.public_key());
    let (key, nonce) = key_schedule(&ss, info);
    let mut out = enc.to_vec();
    out.extend_from_slice(&chacha_seal(&key, &nonce, aad, plaintext));
    Ok(out)
}

/// Opens what [`seal`] produced, as `recipient`, verifying it came from the
/// holder of `sender_pk`.
pub fn open(recipient: &dyn DeviceKey, sender_pk: &[u8; 32], info: &[u8], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < ENC_LEN {
        return Err(CryptoError::Length);
    }
    let (enc, ciphertext) = sealed.split_at(ENC_LEN);
    let enc: [u8; 32] = enc.try_into().expect("ENC_LEN bytes");
    let dh_e = contributory(recipient.dh(&enc))?;
    let dh_s = contributory(recipient.dh(sender_pk))?;
    let mut dh = dh_e.to_vec();
    dh.extend_from_slice(&dh_s);
    let ss = shared_secret(&dh, &enc, &recipient.public_key(), sender_pk);
    let (key, nonce) = key_schedule(&ss, info);
    chacha_open(&key, &nonce, aad, ciphertext)
}

#[cfg(test)]
mod tests {
    use hpke::aead::ChaCha20Poly1305;
    use hpke::kdf::HkdfSha256;
    use hpke::kem::X25519HkdfSha256;
    use hpke::{Deserializable, OpModeR, OpModeS, Serializable};

    use super::*;
    use crate::shell::SoftwareDeviceKey;

    type Priv = <X25519HkdfSha256 as hpke::Kem>::PrivateKey;
    type Pub = <X25519HkdfSha256 as hpke::Kem>::PublicKey;

    fn keys(seed: u8) -> ([u8; 32], SoftwareDeviceKey) {
        let secret = [seed; 32];
        let key = SoftwareDeviceKey::from_bytes(secret);
        (secret, key)
    }

    #[test]
    fn round_trip_binds_sender_info_and_aad() {
        let mut rng = CoreRng::from_seed([1; 32]);
        let (_, sender) = keys(2);
        let (_, recipient) = keys(3);
        let (_, other) = keys(4);
        let sealed = seal(&sender, &recipient.public_key(), b"info", b"aad", b"group key", &mut rng).unwrap();
        assert_eq!(sealed.len(), ENC_LEN + 9 + 16);
        assert_eq!(open(&recipient, &sender.public_key(), b"info", b"aad", &sealed).unwrap(), b"group key");
        assert_eq!(open(&recipient, &other.public_key(), b"info", b"aad", &sealed), Err(CryptoError::Authentication), "another sender");
        assert_eq!(open(&other, &sender.public_key(), b"info", b"aad", &sealed), Err(CryptoError::Authentication), "another recipient");
        assert_eq!(open(&recipient, &sender.public_key(), b"infO", b"aad", &sealed), Err(CryptoError::Authentication), "another info");
        assert_eq!(open(&recipient, &sender.public_key(), b"info", b"aaD", &sealed), Err(CryptoError::Authentication), "another aad");
        assert_eq!(open(&recipient, &sender.public_key(), b"info", b"aad", &sealed[..10]), Err(CryptoError::Length));
    }

    #[test]
    fn interoperates_with_the_hpke_crate_both_ways() {
        let mut rng = CoreRng::from_seed([9; 32]);
        let (sk_s_bytes, sender) = keys(5);
        let (sk_r_bytes, recipient) = keys(6);
        let sk_s = Priv::from_bytes(&sk_s_bytes).unwrap();
        let pk_s = Pub::from_bytes(&sender.public_key()).unwrap();
        let sk_r = Priv::from_bytes(&sk_r_bytes).unwrap();
        let pk_r = Pub::from_bytes(&recipient.public_key()).unwrap();

        // The crate seals, the core opens.
        let (enc, ct) = hpke::single_shot_seal::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
            &OpModeS::Auth((sk_s.clone(), pk_s.clone())),
            &pk_r,
            b"info",
            b"from the crate",
            b"aad",
            &mut rng,
        )
        .unwrap();
        let mut sealed = enc.to_bytes().to_vec();
        sealed.extend_from_slice(&ct);
        assert_eq!(open(&recipient, &sender.public_key(), b"info", b"aad", &sealed).unwrap(), b"from the crate");

        // The core seals, the crate opens.
        let sealed = seal(&sender, &recipient.public_key(), b"info", b"aad", b"from the core", &mut rng).unwrap();
        let enc = <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(&sealed[..ENC_LEN]).unwrap();
        let pt = hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
            &OpModeR::Auth(pk_s),
            &sk_r,
            &enc,
            b"info",
            &sealed[ENC_LEN..],
            b"aad",
        )
        .unwrap();
        assert_eq!(pt, b"from the core");
    }

    #[test]
    fn a_small_order_point_is_refused() {
        let mut rng = CoreRng::from_seed([1; 32]);
        let (_, sender) = keys(2);
        assert_eq!(seal(&sender, &[0u8; 32], b"", b"", b"x", &mut rng), Err(CryptoError::NonContributory));
    }
}
