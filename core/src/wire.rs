//! The protobuf wire format, the little of it the core needs on its own:
//! the envelope is written and read here, and the rebase works at this
//! level (a field is a number and bytes, whatever its type). No reflection,
//! no descriptors: a message is its fields in order.

use std::fmt;

/// A field's wire type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireType {
    /// A varint: int32, int64, uint32, uint64, bool, enum.
    Varint = 0,
    /// Eight bytes: fixed64, sfixed64, double.
    Fixed64 = 1,
    /// Length-delimited: string, bytes, an embedded message, a packed field.
    Bytes = 2,
    /// Four bytes: fixed32, sfixed32, float.
    Fixed32 = 5,
}

/// One field as it stands on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A varint's value.
    Varint(u64),
    /// Eight bytes, little-endian as on the wire.
    Fixed64([u8; 8]),
    /// Length-delimited bytes.
    Bytes(Vec<u8>),
    /// Four bytes.
    Fixed32([u8; 4]),
}

impl Value {
    /// The wire type of the value.
    pub fn wire_type(&self) -> WireType {
        match self {
            Value::Varint(_) => WireType::Varint,
            Value::Fixed64(_) => WireType::Fixed64,
            Value::Bytes(_) => WireType::Bytes,
            Value::Fixed32(_) => WireType::Fixed32,
        }
    }
}

/// A field: its number and its value. A repeated field is several fields
/// with the same number, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The field number, 1 or more.
    pub number: u32,
    /// The value.
    pub value: Value,
}

/// Why bytes are not a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended inside a field.
    Truncated,
    /// A varint longer than ten bytes.
    VarintTooLong,
    /// A wire type the format does not have (3, 4, 6, 7).
    WireType(u8),
    /// A field number of zero.
    FieldNumber,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => f.write_str("truncated message"),
            DecodeError::VarintTooLong => f.write_str("varint over ten bytes"),
            DecodeError::WireType(t) => write!(f, "unknown wire type {t}"),
            DecodeError::FieldNumber => f.write_str("field number zero"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Appends a varint.
pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(input: &[u8], pos: &mut usize) -> Result<u64, DecodeError> {
    let mut v: u64 = 0;
    for i in 0..10 {
        let Some(&b) = input.get(*pos) else { return Err(DecodeError::Truncated) };
        *pos += 1;
        v |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(DecodeError::VarintTooLong)
}

/// Appends one field.
pub fn put_field(out: &mut Vec<u8>, number: u32, value: &Value) {
    put_varint(out, (u64::from(number) << 3) | value.wire_type() as u64);
    match value {
        Value::Varint(v) => put_varint(out, *v),
        Value::Fixed64(b) => out.extend_from_slice(b),
        Value::Bytes(b) => {
            put_varint(out, b.len() as u64);
            out.extend_from_slice(b);
        }
        Value::Fixed32(b) => out.extend_from_slice(b),
    }
}

/// Encodes fields in the order given.
pub fn encode(fields: &[Field]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields {
        put_field(&mut out, f.number, &f.value);
    }
    out
}

/// Decodes a message into its fields, in wire order.
pub fn decode(input: &[u8]) -> Result<Vec<Field>, DecodeError> {
    let mut fields = Vec::new();
    let mut pos = 0;
    while pos < input.len() {
        let tag = get_varint(input, &mut pos)?;
        let number = u32::try_from(tag >> 3).map_err(|_| DecodeError::FieldNumber)?;
        if number == 0 {
            return Err(DecodeError::FieldNumber);
        }
        let value = match (tag & 7) as u8 {
            0 => Value::Varint(get_varint(input, &mut pos)?),
            1 => Value::Fixed64(take::<8>(input, &mut pos)?),
            2 => {
                let len = get_varint(input, &mut pos)? as usize;
                let end = pos.checked_add(len).ok_or(DecodeError::Truncated)?;
                let bytes = input.get(pos..end).ok_or(DecodeError::Truncated)?.to_vec();
                pos = end;
                Value::Bytes(bytes)
            }
            5 => Value::Fixed32(take::<4>(input, &mut pos)?),
            other => return Err(DecodeError::WireType(other)),
        };
        fields.push(Field { number, value });
    }
    Ok(fields)
}

fn take<const N: usize>(input: &[u8], pos: &mut usize) -> Result<[u8; N], DecodeError> {
    let end = pos.checked_add(N).ok_or(DecodeError::Truncated)?;
    let slice = input.get(*pos..end).ok_or(DecodeError::Truncated)?;
    *pos = end;
    let mut out = [0u8; N];
    out.copy_from_slice(slice);
    Ok(out)
}

/// The first field with the number, as a varint.
pub fn varint_of(fields: &[Field], number: u32) -> Option<u64> {
    fields.iter().find(|f| f.number == number).and_then(|f| match &f.value {
        Value::Varint(v) => Some(*v),
        _ => None,
    })
}

/// The first field with the number, as bytes.
pub fn bytes_of(fields: &[Field], number: u32) -> Option<&[u8]> {
    fields.iter().find(|f| f.number == number).and_then(|f| match &f.value {
        Value::Bytes(b) => Some(b.as_slice()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_in_order() {
        let fields = vec![
            Field { number: 1, value: Value::Varint(300) },
            Field { number: 2, value: Value::Bytes(b"hi".to_vec()) },
            Field { number: 2, value: Value::Bytes(Vec::new()) },
            Field { number: 3, value: Value::Fixed64(1u64.to_le_bytes()) },
            Field { number: 16, value: Value::Fixed32(7u32.to_le_bytes()) },
            Field { number: 1 << 20, value: Value::Varint(u64::MAX) },
        ];
        let bytes = encode(&fields);
        assert_eq!(decode(&bytes).unwrap(), fields);
        assert_eq!(&bytes[..3], &[0x08, 0xac, 0x02], "300 as a varint after tag 1");
        assert_eq!(varint_of(&fields, 1), Some(300));
        assert_eq!(bytes_of(&fields, 2), Some(&b"hi"[..]));
    }

    #[test]
    fn malformed_input_is_refused() {
        assert_eq!(decode(&[0x08]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x12, 0x05, 0x01]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x0b]), Err(DecodeError::WireType(3)));
        assert_eq!(decode(&[0x00, 0x00]), Err(DecodeError::FieldNumber));
        assert_eq!(decode(&[0x08, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]), Err(DecodeError::VarintTooLong));
        assert!(decode(&[]).unwrap().is_empty());
    }
}
