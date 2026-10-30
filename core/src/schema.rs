//! The shape of a value: the object types with their numbers, strategies
//! and parent rules; the field kinds the rebase needs (a singular field, or
//! elements by key); the reading and writing of typed fields over
//! [`crate::wire`]; the common fields every object carries; and the
//! `Document` that seals a typed body into an [`Envelope`]
//! (`docs/01-deappje/06-object-types.md`).
//!
//! Field numbers 1 to 9 are the common fields ([`Meta`]); a type's own
//! fields start at 10. A reader ignores numbers it does not know and never
//! writes to an object whose schema version is above its own
//! ([`ObjectType::may_write`]), so no field is ever dropped by a
//! re-serialise.

use std::fmt;

use uuid::Uuid;

use crate::crypto::CryptoError;
use crate::envelope::{Envelope, Header};
use crate::keyring::GroupKey;
use crate::rng::CoreRng;
use crate::wire::{self, DecodeError, Field, Value};

/// How a conflict on an object of the type is resolved
/// (`06-object-types.md`, "Merge strategy per type").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// The changed fields, and for repeated fields the changed elements,
    /// are re-applied onto the current row; the same field changed on both
    /// sides goes to the later push.
    Rebase,
    /// Never edited after creation, so never in conflict but on a stale
    /// delete.
    AppendOnly,
    /// One object per user, only its owner writes: a conflict is between
    /// that user's devices, and the later push wins whole.
    PerUserSingleton,
}

/// Whether an object of the type has a parent (`parent_id` in its value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentRule {
    /// A root: never a parent.
    Never,
    /// A root or a child (a `message` in a thread).
    Optional,
    /// Always a child.
    Required,
}

macro_rules! object_types {
    ($( $(#[$m:meta])* $variant:ident = $num:literal, $name:literal, $strategy:ident, $parent:ident; )*) => {
        /// The object types, by number.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u32)]
        pub enum ObjectType {
            $( $(#[$m])* $variant = $num, )*
        }

        impl ObjectType {
            /// Every type, ascending.
            pub const ALL: &'static [ObjectType] = &[ $( ObjectType::$variant, )* ];

            /// The type's number, what the envelope header carries.
            pub fn number(self) -> u32 {
                self as u32
            }

            /// The type of a number, if it is one.
            pub fn from_number(n: u32) -> Option<ObjectType> {
                match n {
                    $( $num => Some(ObjectType::$variant), )*
                    _ => None,
                }
            }

            /// The type's name as the plan writes it.
            pub fn name(self) -> &'static str {
                match self {
                    $( ObjectType::$variant => $name, )*
                }
            }

            /// How its conflicts resolve.
            pub fn strategy(self) -> Strategy {
                match self {
                    $( ObjectType::$variant => Strategy::$strategy, )*
                }
            }

            /// Whether it has a parent.
            pub fn parent(self) -> ParentRule {
                match self {
                    $( ObjectType::$variant => ParentRule::$parent, )*
                }
            }
        }
    };
}

object_types! {
    /// The group's name, description, avatar and settings; one per group.
    GroupProfile = 1, "group_profile", Rebase, Never;
    /// A member's name, avatar and birthday; one per member.
    MemberProfile = 2, "member_profile", Rebase, Never;
    /// An invite's secret, so any holder can verify a key request that came
    /// through it.
    InviteSecret = 3, "invite_secret", AppendOnly, Never;
    /// A timeline entry.
    Post = 10, "post", Rebase, Never;
    /// A photo under a post, an album or a message.
    Photo = 11, "photo", Rebase, Required;
    /// An album.
    Album = 12, "album", Rebase, Never;
    /// A comment under anything.
    Comment = 13, "comment", Rebase, Required;
    /// A user's reaction on anything; one per user per target.
    Reaction = 14, "reaction", PerUserSingleton, Required;
    /// A todo list.
    TodoList = 20, "todo_list", Rebase, Never;
    /// An item of a todo list.
    TodoItem = 21, "todo_item", Rebase, Required;
    /// A note: its title; the body is its `note_update` children.
    Note = 30, "note", Rebase, Never;
    /// One `yrs` update of a note, from one editing session.
    NoteUpdate = 31, "note_update", AppendOnly, Required;
    /// An expense in a pool.
    Expense = 40, "expense", Rebase, Required;
    /// A payment made outside the app.
    Settlement = 41, "settlement", AppendOnly, Required;
    /// The ticket file of one participant of an expense.
    TicketShare = 42, "ticket_share", Rebase, Required;
    /// A pool of expenses settled independently.
    ExpensePool = 43, "expense_pool", Rebase, Never;
    /// A calendar event.
    Event = 50, "event", Rebase, Never;
    /// A poll, or the datepicker.
    Poll = 60, "poll", Rebase, Never;
    /// A user's answers on a poll; one per user.
    Vote = 61, "vote", PerUserSingleton, Required;
    /// A chat message, or a reply in a thread.
    Message = 70, "message", Rebase, Optional;
    /// A venue recommendation.
    Venue = 80, "venue", Rebase, Never;
}

impl ObjectType {
    /// Whether this client may write an object whose header carries
    /// `row_version`: only when the writer's schema is not newer than its
    /// own, so a re-serialise drops no field.
    pub fn may_write(self, row_version: u32) -> bool {
        row_version <= self.schema().version
    }
}

/// What the rebase needs to know about a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// One value: changed as a whole.
    Singular,
    /// Repeated, each element identified by a key: changed per element.
    Elements(ElementKey),
}

/// How an element of a repeated field is identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKey {
    /// The element is its own key (a list of ids).
    Value,
    /// The element is a message and this sub-field is its key.
    SubField(u32),
}

/// One field of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    /// The field number.
    pub number: u32,
    /// The field's name, for messages to the user.
    pub name: &'static str,
    /// Its kind.
    pub kind: FieldKind,
}

/// A type's schema: its version and its own fields (the common ones are
/// [`Meta::FIELDS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schema {
    /// The schema version this client writes.
    pub version: u32,
    /// The type's own fields.
    pub fields: &'static [FieldSpec],
}

impl Schema {
    /// The kind of a field number, common fields included; a number the
    /// schema does not know is singular.
    pub fn kind(&self, number: u32) -> FieldKind {
        Meta::FIELDS.iter().chain(self.fields.iter()).find(|f| f.number == number).map_or(FieldKind::Singular, |f| f.kind)
    }
}

/// Why bytes are not a value of the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// Not a protobuf message.
    Wire(DecodeError),
    /// A required field is absent.
    Missing(u32),
    /// A field has another wire type than the schema says.
    Type(u32),
    /// A string field is not UTF-8.
    Utf8(u32),
    /// An id field is not 16 bytes, or a message field is too long.
    Length(u32),
    /// A number does not fit its field.
    Range(u32),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Wire(e) => write!(f, "value: {e}"),
            SchemaError::Missing(n) => write!(f, "value: field {n} is missing"),
            SchemaError::Type(n) => write!(f, "value: field {n} has the wrong wire type"),
            SchemaError::Utf8(n) => write!(f, "value: field {n} is not UTF-8"),
            SchemaError::Length(n) => write!(f, "value: field {n} has the wrong length"),
            SchemaError::Range(n) => write!(f, "value: field {n} is out of range"),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<DecodeError> for SchemaError {
    fn from(e: DecodeError) -> Self {
        SchemaError::Wire(e)
    }
}

/// A value that breaks its type's invariants: shown to the user, never
/// pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub &'static str);

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Invalid {}

/// A Rust value that reads from and writes to one field number.
pub trait FieldValue: Sized {
    /// What the rebase treats the field as.
    const KIND: FieldKind;
    /// The value of field `number`, or its default when absent.
    fn read(fields: &[Field], number: u32) -> Result<Self, SchemaError>;
    /// Appends the field, when it is not its default.
    fn write(&self, number: u32, out: &mut Vec<Field>);
}

/// A scalar: one wire value, with proto3's default when absent.
pub trait Scalar: Sized + Default + PartialEq {
    /// From the wire value; `number` names the field in an error.
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError>;
    /// To the wire value.
    fn to_value(&self) -> Value;
}

fn last(fields: &[Field], number: u32) -> Option<&Value> {
    fields.iter().rfind(|f| f.number == number).map(|f| &f.value)
}

/// `FieldValue` for a scalar and for `Option` of it (explicit presence:
/// `Some` is always written, `None` never).
macro_rules! scalar_field {
    ($ty:ty) => {
        impl FieldValue for $ty {
            const KIND: FieldKind = FieldKind::Singular;

            fn read(fields: &[Field], number: u32) -> Result<Self, SchemaError> {
                last(fields, number).map_or(Ok(<$ty>::default()), |v| <$ty as Scalar>::from_value(v, number))
            }

            fn write(&self, number: u32, out: &mut Vec<Field>) {
                if *self != <$ty>::default() {
                    out.push(Field { number, value: <$ty as Scalar>::to_value(self) });
                }
            }
        }

        impl FieldValue for Option<$ty> {
            const KIND: FieldKind = FieldKind::Singular;

            fn read(fields: &[Field], number: u32) -> Result<Self, SchemaError> {
                last(fields, number).map(|v| <$ty as Scalar>::from_value(v, number)).transpose()
            }

            fn write(&self, number: u32, out: &mut Vec<Field>) {
                if let Some(v) = self {
                    out.push(Field { number, value: <$ty as Scalar>::to_value(v) });
                }
            }
        }
    };
}

impl Scalar for String {
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        match value {
            Value::Bytes(b) => String::from_utf8(b.clone()).map_err(|_| SchemaError::Utf8(number)),
            _ => Err(SchemaError::Type(number)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Bytes(self.as_bytes().to_vec())
    }
}

impl Scalar for Vec<u8> {
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        match value {
            Value::Bytes(b) => Ok(b.clone()),
            _ => Err(SchemaError::Type(number)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Bytes(self.clone())
    }
}

impl Scalar for i64 {
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        match value {
            Value::Varint(v) => Ok(*v as i64),
            _ => Err(SchemaError::Type(number)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Varint(*self as u64)
    }
}

impl Scalar for i32 {
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        match value {
            // int32 on the wire is a varint sign-extended to 64 bits.
            Value::Varint(v) => Ok(*v as i64 as i32),
            _ => Err(SchemaError::Type(number)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Varint(i64::from(*self) as u64)
    }
}

impl Scalar for u32 {
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        match value {
            Value::Varint(v) => u32::try_from(*v).map_err(|_| SchemaError::Range(number)),
            _ => Err(SchemaError::Type(number)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Varint(u64::from(*self))
    }
}

impl Scalar for bool {
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        match value {
            Value::Varint(v) => Ok(*v != 0),
            _ => Err(SchemaError::Type(number)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Varint(u64::from(*self))
    }
}

impl Scalar for f64 {
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        match value {
            Value::Fixed64(b) => Ok(f64::from_le_bytes(*b)),
            _ => Err(SchemaError::Type(number)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Fixed64(self.to_le_bytes())
    }
}

scalar_field!(String);
scalar_field!(Vec<u8>);
scalar_field!(i64);
scalar_field!(i32);
scalar_field!(u32);
scalar_field!(bool);
scalar_field!(f64);

fn uuid_of(value: &Value, number: u32) -> Result<Uuid, SchemaError> {
    match value {
        Value::Bytes(b) => Uuid::from_slice(b).map_err(|_| SchemaError::Length(number)),
        _ => Err(SchemaError::Type(number)),
    }
}

/// An id is required where it is not optional: there is no default id.
impl FieldValue for Uuid {
    const KIND: FieldKind = FieldKind::Singular;

    fn read(fields: &[Field], number: u32) -> Result<Self, SchemaError> {
        last(fields, number).ok_or(SchemaError::Missing(number)).and_then(|v| uuid_of(v, number))
    }

    fn write(&self, number: u32, out: &mut Vec<Field>) {
        out.push(Field { number, value: Value::Bytes(self.as_bytes().to_vec()) });
    }
}

impl FieldValue for Option<Uuid> {
    const KIND: FieldKind = FieldKind::Singular;

    fn read(fields: &[Field], number: u32) -> Result<Self, SchemaError> {
        last(fields, number).map(|v| uuid_of(v, number)).transpose()
    }

    fn write(&self, number: u32, out: &mut Vec<Field>) {
        if let Some(id) = self {
            id.write(number, out);
        }
    }
}

/// A protobuf message: its fields, read and written in schema order.
pub trait Message: Sized {
    /// The fields, in number order.
    const FIELDS: &'static [FieldSpec];
    /// From decoded fields; numbers the schema does not know are ignored.
    fn from_fields(fields: &[Field]) -> Result<Self, SchemaError>;
    /// The fields, defaults left out.
    fn to_fields(&self) -> Vec<Field>;
    /// The wire bytes.
    fn encode(&self) -> Vec<u8> {
        wire::encode(&self.to_fields())
    }
    /// From the wire bytes.
    fn decode(bytes: &[u8]) -> Result<Self, SchemaError> {
        Self::from_fields(&wire::decode(bytes)?)
    }
}

/// A nested message is a length-delimited field, present or not.
impl<M: Message> FieldValue for Option<M> {
    const KIND: FieldKind = FieldKind::Singular;

    fn read(fields: &[Field], number: u32) -> Result<Self, SchemaError> {
        match last(fields, number) {
            None => Ok(None),
            Some(Value::Bytes(b)) => M::decode(b).map(Some),
            Some(_) => Err(SchemaError::Type(number)),
        }
    }

    fn write(&self, number: u32, out: &mut Vec<Field>) {
        if let Some(m) = self {
            out.push(Field { number, value: Value::Bytes(m.encode()) });
        }
    }
}

/// An element of a repeated field, identified by a key.
pub trait Element: Sized {
    /// How the rebase identifies one.
    const KEY: ElementKey;
    /// From one occurrence of the field.
    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError>;
    /// To one occurrence of the field.
    fn to_value(&self) -> Value;
}

/// A list of strings (`repeated string`): each is its own key.
impl Element for String {
    const KEY: ElementKey = ElementKey::Value;

    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        <String as Scalar>::from_value(value, number)
    }

    fn to_value(&self) -> Value {
        Scalar::to_value(self)
    }
}

/// A list of ids: each id is its own key.
impl Element for Uuid {
    const KEY: ElementKey = ElementKey::Value;

    fn from_value(value: &Value, number: u32) -> Result<Self, SchemaError> {
        uuid_of(value, number)
    }

    fn to_value(&self) -> Value {
        Value::Bytes(self.as_bytes().to_vec())
    }
}

impl<E: Element> FieldValue for Vec<E> {
    const KIND: FieldKind = FieldKind::Elements(E::KEY);

    fn read(fields: &[Field], number: u32) -> Result<Self, SchemaError> {
        fields.iter().filter(|f| f.number == number).map(|f| E::from_value(&f.value, number)).collect()
    }

    fn write(&self, number: u32, out: &mut Vec<Field>) {
        for e in self {
            out.push(Field { number, value: e.to_value() });
        }
    }
}

/// Declares a message: a struct whose fields map to numbers. With `key N`
/// the message is an element of a repeated field, identified by its
/// sub-field `N`.
#[macro_export]
macro_rules! message {
    ($(#[$m:meta])* $name:ident $(, key $key:literal)? { $( $(#[$fm:meta])* $num:literal => $field:ident : $ty:ty ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
        #[allow(missing_docs)] // a field without its own line is what the plan's table names it
        pub struct $name {
            $( $(#[$fm])* pub $field: $ty, )*
        }

        impl $crate::schema::Message for $name {
            const FIELDS: &'static [$crate::schema::FieldSpec] = &[
                $( $crate::schema::FieldSpec { number: $num, name: stringify!($field), kind: <$ty as $crate::schema::FieldValue>::KIND }, )*
            ];

            #[allow(unused_variables)] // an empty message reads nothing
            fn from_fields(fields: &[$crate::wire::Field]) -> Result<Self, $crate::schema::SchemaError> {
                Ok(Self {
                    $( $field: <$ty as $crate::schema::FieldValue>::read(fields, $num)?, )*
                })
            }

            #[allow(unused_mut)] // an empty message writes nothing
            fn to_fields(&self) -> Vec<$crate::wire::Field> {
                let mut out = Vec::new();
                $( $crate::schema::FieldValue::write(&self.$field, $num, &mut out); )*
                out
            }
        }

        $(
            impl $crate::schema::Element for $name {
                const KEY: $crate::schema::ElementKey = $crate::schema::ElementKey::SubField($key);

                fn from_value(value: &$crate::wire::Value, number: u32) -> Result<Self, $crate::schema::SchemaError> {
                    match value {
                        $crate::wire::Value::Bytes(b) => <Self as $crate::schema::Message>::decode(b),
                        _ => Err($crate::schema::SchemaError::Type(number)),
                    }
                }

                fn to_value(&self) -> $crate::wire::Value {
                    $crate::wire::Value::Bytes($crate::schema::Message::encode(self))
                }
            }
        )?
    };
}

/// Declares an enum carried as a varint. The `#[default]` variant is what
/// an absent field and a value this client does not know both read as: a
/// newer writer's variant is shown as the default rather than refused,
/// and this client never writes to a newer schema. The variants are
/// declared in wire order from 0 (the bindings' derive takes no
/// discriminant from a macro); the numbers written are checked at compile
/// time.
#[macro_export]
macro_rules! wire_enum {
    ($(#[$m:meta])* pub enum $name:ident { $( $(#[$vm:meta])* $variant:ident = $num:literal ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
        #[repr(u32)]
        #[allow(missing_docs)] // a variant without its own line is its name
        pub enum $name {
            $( $(#[$vm])* $variant, )*
        }

        const _: () = {
            $( assert!($name::$variant as u64 == $num, "a wire enum's variants are declared in wire order"); )*
        };

        impl $name {
            /// The variant of a number, or the default for one this client
            /// does not know.
            pub fn from_number(n: u64) -> $name {
                match n {
                    $( $num => $name::$variant, )*
                    _ => $name::default(),
                }
            }
        }

        impl $crate::schema::Scalar for $name {
            fn from_value(value: &$crate::wire::Value, number: u32) -> Result<Self, $crate::schema::SchemaError> {
                match value {
                    $crate::wire::Value::Varint(v) => Ok($name::from_number(*v)),
                    _ => Err($crate::schema::SchemaError::Type(number)),
                }
            }

            fn to_value(&self) -> $crate::wire::Value {
                $crate::wire::Value::Varint(*self as u64)
            }
        }

        impl $crate::schema::FieldValue for $name {
            const KIND: $crate::schema::FieldKind = $crate::schema::FieldKind::Singular;

            fn read(fields: &[$crate::wire::Field], number: u32) -> Result<Self, $crate::schema::SchemaError> {
                match fields.iter().rfind(|f| f.number == number) {
                    None => Ok($name::default()),
                    Some(f) => <$name as $crate::schema::Scalar>::from_value(&f.value, number),
                }
            }

            fn write(&self, number: u32, out: &mut Vec<$crate::wire::Field>) {
                if *self != $name::default() {
                    out.push($crate::wire::Field { number, value: $crate::schema::Scalar::to_value(self) });
                }
            }
        }
    };
}

/// A message that is an object's body: it has a type, a schema version and
/// invariants.
pub trait Object: Message {
    /// The type.
    const TYPE: ObjectType;
    /// The schema version this client writes. Add fields and bump it; never
    /// repurpose a number; a breaking change is a new type.
    const SCHEMA_VERSION: u32 = 1;
    /// The schema, for the rebase.
    const SCHEMA: Schema = Schema { version: Self::SCHEMA_VERSION, fields: Self::FIELDS };

    /// The type's invariants over the body alone.
    fn validate(&self) -> Result<(), Invalid> {
        Ok(())
    }
}

message! {
    #[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
    /// The common fields of every object: numbers 1 to 9.
    Meta {
        /// The parent object, inside the ciphertext: the server sees no
        /// parents.
        1 => parent_id: Option<Uuid>,
        /// The user who created the object.
        2 => author: Uuid,
        /// When it was created, milliseconds since the epoch.
        3 => authored_at: i64,
        /// When it was last edited; the creation time until then.
        4 => edited_at: i64,
    }
}

/// A typed object: its common fields and its body, one flat message.
#[derive(Debug, Clone, PartialEq)]
pub struct Document<T> {
    /// The common fields.
    pub meta: Meta,
    /// The type's own fields.
    pub body: T,
}

/// Why an envelope did not open into a document of the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The header names another type.
    Type {
        /// The type asked for.
        expected: ObjectType,
        /// The number in the header.
        found: u32,
    },
    /// The ciphertext did not open under the key.
    Crypto(CryptoError),
    /// The plaintext is not a value of the type.
    Schema(SchemaError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Type { expected, found } => write!(f, "expected a {} (type {}), found type {found}", expected.name(), expected.number()),
            OpenError::Crypto(e) => write!(f, "{e}"),
            OpenError::Schema(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl<T: Object> Document<T> {
    /// A new object by `author` at `now_ms`, with no parent.
    pub fn new(author: Uuid, now_ms: i64, body: T) -> Document<T> {
        Document { meta: Meta { parent_id: None, author, authored_at: now_ms, edited_at: now_ms }, body }
    }

    /// A new child of `parent_id`.
    pub fn child_of(parent_id: Uuid, author: Uuid, now_ms: i64, body: T) -> Document<T> {
        Document { meta: Meta { parent_id: Some(parent_id), author, authored_at: now_ms, edited_at: now_ms }, body }
    }

    /// The flat fields: the common ones, then the body's.
    pub fn to_fields(&self) -> Vec<Field> {
        let mut fields = self.meta.to_fields();
        fields.extend(self.body.to_fields());
        fields
    }

    /// From flat fields.
    pub fn from_fields(fields: &[Field]) -> Result<Document<T>, SchemaError> {
        Ok(Document { meta: Meta::from_fields(fields)?, body: T::from_fields(fields)? })
    }

    /// The plaintext bytes.
    pub fn encode(&self) -> Vec<u8> {
        wire::encode(&self.to_fields())
    }

    /// From the plaintext bytes.
    pub fn decode(bytes: &[u8]) -> Result<Document<T>, SchemaError> {
        Self::from_fields(&wire::decode(bytes)?)
    }

    /// The envelope header for a value sealed under `key_id`.
    pub fn header(key_id: u32) -> Header {
        Header { object_type: T::TYPE.number(), schema_version: T::SCHEMA_VERSION, key_id }
    }

    /// Seals the document as the object `object_id` of `group_id` under the
    /// group key of `key_id`.
    pub fn seal(&self, key: &GroupKey, key_id: u32, group_id: Uuid, object_id: Uuid, rng: &mut CoreRng) -> Envelope {
        Envelope::seal(key, group_id, object_id, Self::header(key_id), &self.encode(), rng)
    }

    /// Opens an envelope of this type (the caller picked `key` by the
/// header's key id). A newer schema version reads: fields this client
    /// does not know are ignored.
    pub fn open(envelope: &Envelope, key: &GroupKey, group_id: Uuid, object_id: Uuid) -> Result<Document<T>, OpenError> {
        if envelope.header.object_type != T::TYPE.number() {
            return Err(OpenError::Type { expected: T::TYPE, found: envelope.header.object_type });
        }
        let plaintext = envelope.open(key, group_id, object_id).map_err(OpenError::Crypto)?;
        Self::decode(&plaintext).map_err(OpenError::Schema)
    }

    /// The type's invariants: the parent rule, the times, the body's own.
    pub fn validate(&self) -> Result<(), Invalid> {
        match (T::TYPE.parent(), self.meta.parent_id) {
            (ParentRule::Never, Some(_)) => return Err(Invalid("this object cannot have a parent")),
            (ParentRule::Required, None) => return Err(Invalid("this object needs a parent")),
            _ => {}
        }
        if self.meta.author.is_nil() {
            return Err(Invalid("no author"));
        }
        if self.meta.authored_at <= 0 || self.meta.edited_at < self.meta.authored_at {
            return Err(Invalid("edited before it was created"));
        }
        self.body.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    message! {
        Inner, key 1 {
            1 => id: Uuid,
            2 => label: String,
        }
    }

    wire_enum! {
        pub enum Colour {
            #[default]
            Red = 0,
            Blue = 1,
        }
    }

    message! {
        Sample {
            10 => text: String,
            11 => count: u32,
            12 => maybe: Option<i64>,
            13 => inner: Option<Inner>,
            14 => ids: Vec<Uuid>,
            15 => items: Vec<Inner>,
            16 => colour: Colour,
            17 => ratio: f64,
            18 => flag: bool,
            19 => raw: Vec<u8>,
        }
    }

    impl Object for Sample {
        const TYPE: ObjectType = ObjectType::Post;
    }

    #[test]
    fn fields_round_trip_and_defaults_are_absent() {
        let empty = Sample::default();
        assert!(empty.to_fields().is_empty(), "every default is left out");
        assert_eq!(Sample::decode(&[]).unwrap(), empty);
        let full = Sample {
            text: "hi".into(),
            count: 7,
            maybe: Some(0),
            inner: Some(Inner { id: Uuid::from_u128(1), label: "in".into() }),
            ids: vec![Uuid::from_u128(2), Uuid::from_u128(3)],
            items: vec![Inner { id: Uuid::from_u128(4), label: String::new() }],
            colour: Colour::Blue,
            ratio: 0.5,
            flag: true,
            raw: vec![1, 2],
        };
        let fields = full.to_fields();
        assert_eq!(Sample::from_fields(&fields).unwrap(), full);
        assert_eq!(fields.iter().filter(|f| f.number == 12).count(), 1, "Some(0) is present");
        assert_eq!(Sample::FIELDS[4].kind, FieldKind::Elements(ElementKey::Value));
        assert_eq!(Sample::FIELDS[5].kind, FieldKind::Elements(ElementKey::SubField(1)));
        assert_eq!(Sample::SCHEMA.kind(2), FieldKind::Singular, "a common field");
        assert_eq!(Sample::SCHEMA.kind(15), FieldKind::Elements(ElementKey::SubField(1)));
    }

    #[test]
    fn unknown_fields_and_values_read_as_the_plan_says() {
        let mut fields = Sample { text: "t".into(), ..Sample::default() }.to_fields();
        fields.push(Field { number: 99, value: Value::Bytes(b"from a newer schema".to_vec()) });
        fields.push(Field { number: 16, value: Value::Varint(42) });
        let read = Sample::from_fields(&fields).unwrap();
        assert_eq!(read.text, "t");
        assert_eq!(read.colour, Colour::Red, "an unknown enum value shows as the default");
        assert_eq!(Sample::read_error(&[Field { number: 10, value: Value::Varint(1) }]), SchemaError::Type(10));
        assert_eq!(Sample::read_error(&[Field { number: 10, value: Value::Bytes(vec![0xff]) }]), SchemaError::Utf8(10));
        assert_eq!(Sample::read_error(&[Field { number: 11, value: Value::Varint(1 << 40) }]), SchemaError::Range(11));
        assert_eq!(Sample::read_error(&[Field { number: 14, value: Value::Bytes(vec![1; 3]) }]), SchemaError::Length(14));
        assert_eq!(Inner::from_fields(&[]).unwrap_err(), SchemaError::Missing(1), "an id is required");
    }

    impl Sample {
        fn read_error(fields: &[Field]) -> SchemaError {
            Sample::from_fields(fields).unwrap_err()
        }
    }

    #[test]
    fn a_document_seals_opens_and_checks_its_type() {
        let mut rng = CoreRng::from_seed([2; 32]);
        let key = GroupKey::mint(&mut rng);
        let (g, o, author) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let doc = Document::new(author, 1_000, Sample { text: "hello".into(), ..Sample::default() });
        assert!(doc.validate().is_ok());
        let env = doc.seal(&key, 3, g, o, &mut rng);
        assert_eq!(env.header, Header { object_type: 10, schema_version: 1, key_id: 3 });
        assert_eq!(Document::<Sample>::open(&env, &key, g, o).unwrap(), doc);
        let relabelled = Envelope { header: Header { object_type: 11, ..env.header }, ..env.clone() };
        assert_eq!(Document::<Sample>::open(&relabelled, &key, g, o), Err(OpenError::Type { expected: ObjectType::Post, found: 11 }));
        assert!(matches!(Document::<Sample>::open(&env, &GroupKey::mint(&mut rng), g, o), Err(OpenError::Crypto(_))));
        assert_eq!(Document::<Sample>::child_of(o, author, 5, Sample::default()).validate(), Err(Invalid("this object cannot have a parent")));
        let mut late = doc.clone();
        late.meta.edited_at = 999;
        assert_eq!(late.validate(), Err(Invalid("edited before it was created")));
    }

    #[test]
    fn types_are_numbered_as_the_plan_has_them() {
        assert_eq!(ObjectType::ALL.len(), 21);
        assert_eq!(ObjectType::from_number(42), Some(ObjectType::TicketShare));
        assert_eq!(ObjectType::from_number(4), None);
        assert_eq!(ObjectType::Vote.strategy(), Strategy::PerUserSingleton);
        assert_eq!(ObjectType::Message.parent(), ParentRule::Optional);
        assert_eq!(ObjectType::NoteUpdate.name(), "note_update");
        assert!(ObjectType::Post.may_write(1));
        assert!(!ObjectType::Post.may_write(2), "a newer writer's object is read, never written");
    }
}
