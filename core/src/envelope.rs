//! The value of an object as it crosses the wire and sits in the relay: a
//! plaintext header (the type, the schema version, the key id) the reader
//! needs before it can pick a key, then the nonce and the ciphertext. The
//! header is the AEAD's associated data together with the group id and the
//! object id, so a header cannot be moved onto another ciphertext nor a
//! ciphertext onto another object (`docs/01-deappje/06-object-types.md`).
//!
//! On the wire it is a protobuf message, `Envelope { uint32 type = 1;
//! uint32 schema_version = 2; uint32 key_id = 3; bytes nonce = 4; bytes
//! ciphertext = 5; }`, written and read through [`crate::wire`].

use std::fmt;

use uuid::Uuid;

use crate::crypto::CryptoError;
use crate::crypto::aead::{xchacha_open, xchacha_seal};
use crate::keyring::GroupKey;
use crate::rng::CoreRng;
use crate::wire::{self, Field, Value};

const F_TYPE: u32 = 1;
const F_SCHEMA_VERSION: u32 = 2;
const F_KEY_ID: u32 = 3;
const F_NONCE: u32 = 4;
const F_CIPHERTEXT: u32 = 5;

/// What a reader sees before decrypting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// The object type's number (`06-object-types.md`).
    pub object_type: u32,
    /// The schema version the writer used.
    pub schema_version: u32,
    /// The group key generation the value is sealed under.
    pub key_id: u32,
}

impl Header {
    fn fields(&self) -> Vec<Field> {
        vec![
            Field { number: F_TYPE, value: Value::Varint(u64::from(self.object_type)) },
            Field { number: F_SCHEMA_VERSION, value: Value::Varint(u64::from(self.schema_version)) },
            Field { number: F_KEY_ID, value: Value::Varint(u64::from(self.key_id)) },
        ]
    }

    /// The associated data: the group, the object, the header.
    pub fn aad(&self, group_id: Uuid, object_id: Uuid) -> Vec<u8> {
        let mut aad = group_id.as_bytes().to_vec();
        aad.extend_from_slice(object_id.as_bytes());
        aad.extend_from_slice(&wire::encode(&self.fields()));
        aad
    }
}

/// A sealed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// The plaintext header.
    pub header: Header,
    /// The XChaCha20-Poly1305 nonce.
    pub nonce: [u8; 24],
    /// The sealed message bytes.
    pub ciphertext: Vec<u8>,
}

/// Why bytes are not an envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    /// Not a protobuf message.
    Wire(wire::DecodeError),
    /// A header field is missing or does not fit.
    Header,
    /// The nonce is not 24 bytes.
    Nonce,
    /// The ciphertext is absent.
    Ciphertext,
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvelopeError::Wire(e) => write!(f, "envelope: {e}"),
            EnvelopeError::Header => f.write_str("envelope: header incomplete"),
            EnvelopeError::Nonce => f.write_str("envelope: nonce is not 24 bytes"),
            EnvelopeError::Ciphertext => f.write_str("envelope: no ciphertext"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

impl Envelope {
    /// Seals `plaintext` for the object under the group key.
    pub fn seal(key: &GroupKey, group_id: Uuid, object_id: Uuid, header: Header, plaintext: &[u8], rng: &mut CoreRng) -> Envelope {
        let nonce = rng.nonce24();
        let ciphertext = xchacha_seal(key.bytes(), &nonce, &header.aad(group_id, object_id), plaintext);
        Envelope { header, nonce, ciphertext }
    }

    /// Opens the value as the object's, under the group key the header
    /// names (the caller picked it by `header.key_id`).
    pub fn open(&self, key: &GroupKey, group_id: Uuid, object_id: Uuid) -> Result<Vec<u8>, CryptoError> {
        xchacha_open(key.bytes(), &self.nonce, &self.header.aad(group_id, object_id), &self.ciphertext)
    }

    /// The wire bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut fields = self.header.fields();
        fields.push(Field { number: F_NONCE, value: Value::Bytes(self.nonce.to_vec()) });
        fields.push(Field { number: F_CIPHERTEXT, value: Value::Bytes(self.ciphertext.clone()) });
        wire::encode(&fields)
    }

    /// The header alone, from the wire bytes: what a reader needs to pick
    /// a key, and what any device can read without one.
    pub fn peek(bytes: &[u8]) -> Result<Header, EnvelopeError> {
        let fields = wire::decode(bytes).map_err(EnvelopeError::Wire)?;
        header_of(&fields)
    }

    /// From the wire bytes.
    pub fn decode(bytes: &[u8]) -> Result<Envelope, EnvelopeError> {
        let fields = wire::decode(bytes).map_err(EnvelopeError::Wire)?;
        let header = header_of(&fields)?;
        let nonce: [u8; 24] = wire::bytes_of(&fields, F_NONCE)
            .ok_or(EnvelopeError::Nonce)?
            .try_into()
            .map_err(|_| EnvelopeError::Nonce)?;
        let ciphertext = wire::bytes_of(&fields, F_CIPHERTEXT).ok_or(EnvelopeError::Ciphertext)?.to_vec();
        Ok(Envelope { header, nonce, ciphertext })
    }
}

fn header_of(fields: &[Field]) -> Result<Header, EnvelopeError> {
    let small = |n: u32| wire::varint_of(fields, n).and_then(|v| u32::try_from(v).ok()).ok_or(EnvelopeError::Header);
    Ok(Header { object_type: small(F_TYPE)?, schema_version: small(F_SCHEMA_VERSION)?, key_id: small(F_KEY_ID)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seals_opens_and_binds_the_header_to_the_object() {
        let mut rng = CoreRng::from_seed([4; 32]);
        let key = GroupKey::mint(&mut rng);
        let (g, o) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let header = Header { object_type: 10, schema_version: 1, key_id: 1 };
        let env = Envelope::seal(&key, g, o, header, b"a post", &mut rng);
        let bytes = env.encode();
        assert_eq!(Envelope::peek(&bytes).unwrap(), header);
        let back = Envelope::decode(&bytes).unwrap();
        assert_eq!(back, env);
        assert_eq!(back.open(&key, g, o).unwrap(), b"a post");
        assert_eq!(back.open(&key, g, Uuid::from_u128(3)), Err(CryptoError::Authentication), "another object");
        assert_eq!(back.open(&key, Uuid::from_u128(9), o), Err(CryptoError::Authentication), "another group");
        let relabelled = Envelope { header: Header { object_type: 11, ..header }, ..back.clone() };
        assert_eq!(relabelled.open(&key, g, o), Err(CryptoError::Authentication), "a header swapped onto the ciphertext");
        assert_eq!(back.open(&GroupKey::mint(&mut rng), g, o), Err(CryptoError::Authentication));
    }

    #[test]
    fn malformed_envelopes_are_refused() {
        assert_eq!(Envelope::decode(&[0x08]), Err(EnvelopeError::Wire(wire::DecodeError::Truncated)));
        let no_header = wire::encode(&[Field { number: F_NONCE, value: Value::Bytes(vec![0; 24]) }]);
        assert_eq!(Envelope::decode(&no_header), Err(EnvelopeError::Header));
        let short_nonce = wire::encode(&[
            Field { number: F_TYPE, value: Value::Varint(1) },
            Field { number: F_SCHEMA_VERSION, value: Value::Varint(1) },
            Field { number: F_KEY_ID, value: Value::Varint(1) },
            Field { number: F_NONCE, value: Value::Bytes(vec![0; 12]) },
            Field { number: F_CIPHERTEXT, value: Value::Bytes(vec![0; 16]) },
        ]);
        assert_eq!(Envelope::decode(&short_nonce), Err(EnvelopeError::Nonce));
        assert_eq!(Envelope::peek(&short_nonce).unwrap().key_id, 1, "the header reads without the rest");
    }
}
