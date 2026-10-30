//! The core as a shell sees it: one object, [`Core`], holding the device's
//! identity, keys, store and server client behind the seams the shell
//! gave it, with the operations a screen needs (`docs/01-deappje/
//! 01-architecture.md`, "The shells"). On the phones every item here is
//! exported through UniFFI; the browser reaches the same through
//! wasm-bindgen. A shell never parses an object value: it gets and gives
//! [`ObjectBody`], the typed body of one object.
//!
//! One lock guards the state through a whole operation, server calls
//! included: a shell calls the core from one place at a time, and a sync
//! that overlaps a write would otherwise push half of it.

use std::fmt;
use std::sync::Arc;

use async_lock::Mutex;
use uuid::Uuid;

use crate::api::{ApiError, Code};
use crate::connect::{Connect, Membership};
use crate::crypto::CryptoError;
use crate::crypto::aead::{xchacha_open, xchacha_seal};
use crate::crypto::hash::sha256;
use crate::ids;
use crate::invite::{join_link, parse_join_link};
use crate::keyring::{GroupKey, Keyring};
use crate::keys::{KeyError, KeyEvent, KeyExchange};
use crate::kratos::{Identity, Kratos, KratosError, LoginStart};
use crate::rng::CoreRng;
use crate::schema::{Document, Message as _, Meta, ObjectType, SchemaError};
use crate::shell::{Clock, DeviceKey, DeviceKeyExt as _, Random, Secrets, Storage};
use crate::store::{Op, Phase, Store};
use crate::sync::{Event as SyncEvent, SyncError, Syncer, read_plain};
use crate::transport::{Header, Request, Transport};
use crate::trust::{PinMismatch, fingerprint};
use crate::types::*;

/// The secret the device's id lives under.
pub const SECRET_DEVICE_ID: &str = "device_id";
/// The secret the session token lives under.
pub const SECRET_SESSION_TOKEN: &str = "session_token";

/// Why an operation did not go.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Error))]
pub enum CoreError {
    /// The server refused or could not be reached.
    Api {
        /// The code, lowercase as the protocol names it.
        code: String,
        /// The message.
        message: String,
    },
    /// No session: sign in first.
    NotSignedIn,
    /// The device holds no key of the group yet.
    NoKey {
        /// The group.
        group_id: Uuid,
    },
    /// A value its type refuses.
    Invalid {
        /// Why.
        reason: String,
    },
    /// An object the device does not hold.
    Unknown {
        /// The object.
        id: Uuid,
    },
    /// Signing in failed.
    Login {
        /// What Kratos said.
        message: String,
    },
    /// A link that is not an invite link.
    Link,
    /// A cryptographic check failed.
    Crypto {
        /// Which.
        message: String,
    },
    /// A known device offered another key.
    Pin {
        /// The device.
        device_id: Uuid,
    },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Api { code, message } => write!(f, "{code}: {message}"),
            CoreError::NotSignedIn => f.write_str("not signed in"),
            CoreError::NoKey { group_id } => write!(f, "no key of group {group_id}"),
            CoreError::Invalid { reason } => write!(f, "{reason}"),
            CoreError::Unknown { id } => write!(f, "object {id} is not held"),
            CoreError::Login { message } => write!(f, "sign-in: {message}"),
            CoreError::Link => f.write_str("not an invite link"),
            CoreError::Crypto { message } => write!(f, "{message}"),
            CoreError::Pin { device_id } => write!(f, "device {device_id} offers another key"),
        }
    }
}

impl std::error::Error for CoreError {}

fn code_name(code: Code) -> &'static str {
    match code {
        Code::InvalidArgument => "invalid_argument",
        Code::FailedPrecondition => "failed_precondition",
        Code::PermissionDenied => "permission_denied",
        Code::NotFound => "not_found",
        Code::Internal => "internal",
        Code::Unavailable => "unavailable",
    }
}

impl From<ApiError> for CoreError {
    fn from(e: ApiError) -> Self {
        CoreError::Api { code: code_name(e.code).to_owned(), message: e.message }
    }
}

impl From<KratosError> for CoreError {
    fn from(e: KratosError) -> Self {
        match e {
            KratosError::Unauthenticated => CoreError::NotSignedIn,
            other => CoreError::Login { message: other.to_string() },
        }
    }
}

impl From<SyncError> for CoreError {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Relay(e) => e.into(),
            SyncError::NoKey { group_id, .. } | SyncError::NoGroupKey { group_id } => CoreError::NoKey { group_id },
            SyncError::Crypto(e) => CoreError::Crypto { message: e.to_string() },
            SyncError::Envelope(e) => CoreError::Crypto { message: e.to_string() },
            SyncError::Unknown { id } => CoreError::Unknown { id },
        }
    }
}

impl From<KeyError> for CoreError {
    fn from(e: KeyError) -> Self {
        match e {
            KeyError::Api(e) => e.into(),
            KeyError::Pin(m) => CoreError::Pin { device_id: m.device_id },
            KeyError::NoKey { group_id } => CoreError::NoKey { group_id },
            KeyError::PublicKey { .. } | KeyError::Crypto(_) => CoreError::Crypto { message: e.to_string() },
        }
    }
}

impl From<SchemaError> for CoreError {
    fn from(e: SchemaError) -> Self {
        CoreError::Invalid { reason: e.to_string() }
    }
}

impl From<CryptoError> for CoreError {
    fn from(e: CryptoError) -> Self {
        CoreError::Crypto { message: e.to_string() }
    }
}

macro_rules! object_bodies {
    ($( $variant:ident ),* $(,)?) => {
        /// The typed body of an object: one variant per type.
        #[derive(Debug, Clone, PartialEq)]
        #[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Enum))]
        #[derive(serde::Serialize, serde::Deserialize)]
        #[serde(tag = "type", content = "fields")]
        pub enum ObjectBody {
            $( #[allow(missing_docs)] $variant($variant), )*
        }

        impl ObjectBody {
            /// The type.
            pub fn object_type(&self) -> ObjectType {
                match self {
                    $( ObjectBody::$variant(_) => ObjectType::$variant, )*
                }
            }

            /// The body's own fields on the wire, after the common ones.
            fn body_fields(&self) -> Vec<crate::wire::Field> {
                match self {
                    $( ObjectBody::$variant(b) => b.to_fields(), )*
                }
            }

            /// The body's invariants.
            fn validate(&self) -> Result<(), crate::schema::Invalid> {
                match self {
                    $( ObjectBody::$variant(b) => crate::schema::Object::validate(b), )*
                }
            }

            /// A body of the type from decoded fields.
            fn from_fields(object_type: ObjectType, fields: &[crate::wire::Field]) -> Result<ObjectBody, SchemaError> {
                Ok(match object_type {
                    $( ObjectType::$variant => ObjectBody::$variant(<$variant as crate::schema::Message>::from_fields(fields)?), )*
                })
            }
        }
    };
}

object_bodies! {
    GroupProfile, MemberProfile, InviteSecret, Post, Photo, Album, Comment, Reaction, TodoList, TodoItem, Note, NoteUpdate,
    Expense, Settlement, TicketShare, ExpensePool, Event, Poll, Vote, Message, Venue,
}

/// An object as a screen shows it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ObjectView {
    /// The object.
    pub id: Uuid,
    /// Its parent, if any.
    pub parent_id: Option<Uuid>,
    /// Who created it.
    pub author: Uuid,
    /// When.
    pub authored_at_ms: i64,
    /// When it was last edited.
    pub edited_at_ms: i64,
    /// Its seq on the server; 0 while a create is not yet accepted.
    pub seq: i64,
    /// Whether a write of this device is not yet accepted.
    pub pending: bool,
    /// The body.
    pub body: ObjectBody,
}

/// A group as the device knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct GroupView {
    /// The group.
    pub id: Uuid,
    /// Its parent, for a subgroup.
    pub parent_group_id: Option<Uuid>,
    /// Whether the device holds a key: without one it can read nothing.
    pub holds_keys: bool,
    /// `new`, `catching_up` or `holder`: where the device stands in the
    /// group's log.
    pub phase: String,
}

/// What joining through a link did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct JoinResult {
    /// The group joined.
    pub group_id: Uuid,
    /// The key request made, waiting for a holder's approval.
    pub request_id: Uuid,
}

/// A key request as the approving screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct PendingRequestView {
    /// The request.
    pub request_id: Uuid,
    /// The group.
    pub group_id: Uuid,
    /// The device asking.
    pub device_id: Uuid,
    /// Its user.
    pub user_id: Uuid,
    /// Its platform.
    pub platform: String,
    /// What the requesting device shows on its screen.
    pub fingerprint: String,
    /// Whether the invite it came through vouches for it; none when it
    /// came through no invite this device knows.
    pub invite_verified: Option<bool>,
    /// When it was made.
    pub created_at_ms: i64,
}

/// A write parked because its rebase broke the type's invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct InvalidWrite {
    /// The group.
    pub group_id: Uuid,
    /// The object.
    pub object_id: Uuid,
    /// Why.
    pub reason: String,
}

/// A known device offering another public key: shown, never adopted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct PinAlert {
    /// The device.
    pub device_id: Uuid,
    /// The fingerprint pinned.
    pub pinned: String,
    /// The fingerprint offered.
    pub offered: String,
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct SyncReport {
    /// Groups whose rows changed: re-read them.
    pub changed: Vec<Uuid>,
    /// Groups this device now holds a key for that it did not before.
    pub keys_received: Vec<Uuid>,
    /// Groups the device threw away and is fetching again.
    pub started_over: Vec<Uuid>,
    /// Writes waiting for the user.
    pub invalid: Vec<InvalidWrite>,
    /// Known devices offering other keys.
    pub pin_alerts: Vec<PinAlert>,
    /// Groups where a sync failed, with why; the rest went on.
    pub failed: Vec<String>,
}

/// A file stored in the bucket, as [`FileRef`] in its object.
fn blob_aad(group_id: Uuid, object_id: Uuid, slot: &str) -> Vec<u8> {
    let mut aad = group_id.as_bytes().to_vec();
    aad.extend_from_slice(object_id.as_bytes());
    aad.extend_from_slice(slot.as_bytes());
    aad
}

/// The bucket key of a slot.
pub fn blob_key(group_id: Uuid, object_id: Uuid, slot: &str) -> String {
    format!("g/{group_id}/o/{object_id}/{slot}")
}

struct Inner {
    device_id: Uuid,
    user_id: Option<Uuid>,
    device_key: Arc<dyn DeviceKey>,
    clock: Arc<dyn Clock>,
    secrets: Arc<dyn Secrets>,
    storage: Arc<dyn Storage>,
    store: Store,
    keyring: Keyring,
    pins: crate::trust::Pins,
    rng: CoreRng,
    connect: Connect<Arc<dyn Transport>>,
}

fn key_path(group_id: Uuid, key_id: u32) -> String {
    format!("k/{group_id}/{key_id}")
}

fn pin_path(device_id: Uuid) -> String {
    format!("p/{device_id}")
}

impl Inner {
    fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    fn user(&self) -> Result<Uuid, CoreError> {
        self.user_id.ok_or(CoreError::NotSignedIn)
    }

    fn syncer(&mut self) -> Syncer<'_> {
        Syncer { device_id: self.device_id, store: &mut self.store, keyring: &self.keyring, rng: &mut self.rng }
    }

    /// Writes every key the device holds of a group, and every pin.
    fn persist_trust(&self, group_id: Option<Uuid>) {
        let groups = group_id.map_or_else(|| self.keyring.groups(), |g| vec![g]);
        for g in groups {
            for key_id in self.keyring.key_ids(g) {
                if let Some(key) = self.keyring.get(g, key_id) {
                    self.storage.put(key_path(g, key_id), key.bytes().to_vec());
                }
            }
        }
        for (device, pk) in self.pins.iter() {
            self.storage.put(pin_path(*device), pk.to_vec());
        }
    }

    fn load_trust(storage: &dyn Storage, keyring: &mut Keyring, pins: &mut crate::trust::Pins) {
        for key in storage.keys("k/".to_owned()) {
            let parts: Vec<&str> = key.split('/').collect();
            let (Some(g), Some(k)) = (parts.get(1).and_then(|s| s.parse::<Uuid>().ok()), parts.get(2).and_then(|s| s.parse::<u32>().ok())) else { continue };
            if let Some(bytes) = storage.get(key.clone()).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
                keyring.insert(g, k, GroupKey::from_bytes(bytes));
            }
        }
        for key in storage.keys("p/".to_owned()) {
            let Some(d) = key.strip_prefix("p/").and_then(|s| s.parse::<Uuid>().ok()) else { continue };
            if let Some(pk) = storage.get(key.clone()).and_then(|b| <[u8; 32]>::try_from(b).ok()) {
                pins.accept(d, pk);
            }
        }
    }

    /// The invite secrets the device holds, by invite: the groups'
    /// `invite_secret` objects.
    fn invite_secrets(&self) -> Vec<(Uuid, crate::invite::InviteSecret)> {
        let mut out = Vec::new();
        for g in self.store.groups() {
            let Some(group) = self.store.group(g) else { continue };
            for (id, row) in &group.rows {
                if row.deleted || row.object_type != Some(ObjectType::InviteSecret) {
                    continue;
                }
                let Ok(Some(plain)) = read_plain(&self.store, &self.keyring, g, *id) else { continue };
                if let Ok(doc) = Document::<InviteSecret>::decode(&plain)
                    && let Ok(bytes) = <[u8; 16]>::try_from(doc.body.secret.as_slice())
                {
                    out.push((doc.body.invite_id, crate::invite::InviteSecret::from_bytes(bytes)));
                }
            }
        }
        out
    }

    fn view(&self, group_id: Uuid, id: Uuid) -> Option<ObjectView> {
        let row = self.store.row(group_id, id)?;
        if row.deleted {
            return None;
        }
        let pending = self.store.pending(group_id, id);
        let plain = match pending.map(|p| &p.op) {
            Some(Op::Create { plain, .. } | Op::Edit { plain, .. }) => plain.clone(),
            Some(Op::Delete { .. }) => return None,
            None => {
                let value = row.value.as_ref()?;
                let envelope = crate::envelope::Envelope::decode(value).ok()?;
                let key = self.keyring.get(group_id, envelope.header.key_id)?;
                envelope.open(key, group_id, id).ok()?
            }
        };
        let object_type = row.object_type.or_else(|| {
            let value = row.value.as_ref()?;
            ObjectType::from_number(crate::envelope::Envelope::peek(value).ok()?.object_type)
        })?;
        let fields = crate::wire::decode(&plain).ok()?;
        let meta = Meta::from_fields(&fields).ok()?;
        let body = ObjectBody::from_fields(object_type, &fields).ok()?;
        Some(ObjectView { id, parent_id: meta.parent_id, author: meta.author, authored_at_ms: meta.authored_at, edited_at_ms: meta.edited_at, seq: row.seq, pending: pending.is_some(), body })
    }
}

/// The core.
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Object))]
pub struct Core {
    inner: Mutex<Inner>,
}

#[cfg_attr(not(target_arch = "wasm32"), uniffi::export)]
impl Core {
    /// Opens the core over the shell's seams and the deployment's origin
    /// (`https://<host>`). The device id is minted on first use and kept
    /// in the secrets; a kept session token is sent until it is refused.
    #[cfg_attr(not(target_arch = "wasm32"), uniffi::constructor)]
    pub fn new(transport: Arc<dyn Transport>, storage: Arc<dyn Storage>, secrets: Arc<dyn Secrets>, device_key: Arc<dyn DeviceKey>, random: Arc<dyn Random>, clock: Arc<dyn Clock>, base_url: String) -> Arc<Core> {
        let mut rng = CoreRng::from_shell(&*random);
        let device_id = match secrets.get_secret(SECRET_DEVICE_ID.to_owned()).and_then(|s| s.parse().ok()) {
            Some(id) => id,
            None => {
                let id = ids::random(&mut rng);
                secrets.set_secret(SECRET_DEVICE_ID.to_owned(), id.to_string());
                id
            }
        };
        let connect = Connect::new(transport, &base_url);
        connect.set_session_token(secrets.get_secret(SECRET_SESSION_TOKEN.to_owned()));
        let store = Store::open(Box::new(storage.clone()));
        let mut keyring = Keyring::new();
        let mut pins = crate::trust::Pins::new();
        Inner::load_trust(&*storage, &mut keyring, &mut pins);
        Arc::new(Core { inner: Mutex::new(Inner { device_id, user_id: None, device_key, clock, secrets, storage, store, keyring, pins, rng, connect }) })
    }

    /// This device's id.
    pub async fn device_id(&self) -> Uuid {
        self.inner.lock().await.device_id
    }

    /// This device's fingerprint, what the approver reads off its screen.
    pub async fn fingerprint(&self) -> String {
        fingerprint(&self.inner.lock().await.device_key.public_key())
    }

    /// The signed-in user, once known.
    pub async fn user_id(&self) -> Option<Uuid> {
        self.inner.lock().await.user_id
    }

    // ---- Signing in ----

    /// A browser session's cookie carries the session instead of a token:
    /// the web shell passes none.
    pub async fn set_session_token(&self, token: Option<String>) {
        let inner = self.inner.lock().await;
        match &token {
            Some(t) => inner.secrets.set_secret(SECRET_SESSION_TOKEN.to_owned(), t.clone()),
            None => inner.secrets.delete_secret(SECRET_SESSION_TOKEN.to_owned()),
        }
        inner.connect.set_session_token(token);
    }

    /// Who the kept session signs in as, if it still does.
    pub async fn restore_session(&self) -> Result<Option<Identity>, CoreError> {
        let mut inner = self.inner.lock().await;
        let Some(token) = inner.secrets.get_secret(SECRET_SESSION_TOKEN.to_owned()) else { return Ok(None) };
        let kratos = Kratos::new(inner.connect.transport(), inner.connect.base_url());
        match kratos.whoami(&token).await {
            Ok(identity) => {
                inner.user_id = identity.id.parse().ok();
                Ok(Some(identity))
            }
            Err(KratosError::Unauthenticated) => {
                inner.secrets.delete_secret(SECRET_SESSION_TOKEN.to_owned());
                inner.connect.set_session_token(None);
                inner.user_id = None;
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Starts a native sign-in that returns to `return_to`.
    pub async fn start_login(&self, return_to: String) -> Result<LoginStart, CoreError> {
        let inner = self.inner.lock().await;
        let kratos = Kratos::new(inner.connect.transport(), inner.connect.base_url());
        Ok(kratos.start_login(&return_to).await?)
    }

    /// The browser came back with `code`: the session is kept and the
    /// identity known.
    pub async fn finish_login(&self, start: LoginStart, code: String) -> Result<Identity, CoreError> {
        let mut inner = self.inner.lock().await;
        let kratos = Kratos::new(inner.connect.transport(), inner.connect.base_url());
        let token = kratos.finish_login(&start, &code).await?;
        let identity = kratos.whoami(&token).await?;
        inner.secrets.set_secret(SECRET_SESSION_TOKEN.to_owned(), token.clone());
        inner.connect.set_session_token(Some(token));
        inner.user_id = identity.id.parse().ok();
        Ok(identity)
    }

    /// Ends the session.
    pub async fn logout(&self) -> Result<(), CoreError> {
        let mut inner = self.inner.lock().await;
        if let Some(token) = inner.secrets.get_secret(SECRET_SESSION_TOKEN.to_owned()) {
            let kratos = Kratos::new(inner.connect.transport(), inner.connect.base_url());
            let _ = kratos.logout(&token).await;
        }
        inner.secrets.delete_secret(SECRET_SESSION_TOKEN.to_owned());
        inner.connect.set_session_token(None);
        inner.user_id = None;
        Ok(())
    }

    /// Registers this device with its public key (`ios`, `android` or
    /// `web`). A repeat refreshes.
    pub async fn register_device(&self, platform: String) -> Result<(), CoreError> {
        let mut inner = self.inner.lock().await;
        let inner = &mut *inner;
        let mut kx = KeyExchange { device_id: inner.device_id, device_key: &*inner.device_key, keyring: &mut inner.keyring, pins: &mut inner.pins, rng: &mut inner.rng };
        Ok(kx.register(&inner.connect, &platform).await?)
    }

    /// Hands the server the push token (`apns` or `fcm`); empty clears.
    pub async fn set_push_token(&self, provider: String, token: String) -> Result<(), CoreError> {
        let inner = self.inner.lock().await;
        Ok(inner.connect.set_push_token(inner.device_id, &provider, &token).await?)
    }

    // ---- Groups ----

    /// The groups the user is in, with whether this device can read them.
    pub async fn list_groups(&self) -> Result<Vec<GroupView>, CoreError> {
        let inner = self.inner.lock().await;
        let groups = inner.connect.list_groups().await?;
        Ok(groups
            .into_iter()
            .map(|g| GroupView {
                id: g.id,
                parent_group_id: g.parent_group_id,
                holds_keys: inner.keyring.holds(g.id),
                phase: match inner.store.group(g.id).map(|s| &s.phase) {
                    Some(Phase::Holder) => "holder",
                    Some(Phase::CatchingUp { .. }) => "catching_up",
                    _ => "new",
                }
                .to_owned(),
            })
            .collect())
    }

    /// A new group, or a subgroup: created on the server, its first key
    /// minted here. The profile is the shell's first object in it.
    pub async fn create_group(&self, parent_group_id: Option<Uuid>) -> Result<Uuid, CoreError> {
        let mut inner = self.inner.lock().await;
        let inner = &mut *inner;
        let group = inner.connect.create_group(parent_group_id).await?;
        let mut kx = KeyExchange { device_id: inner.device_id, device_key: &*inner.device_key, keyring: &mut inner.keyring, pins: &mut inner.pins, rng: &mut inner.rng };
        kx.create_group_key(&inner.connect, group.id).await?;
        inner.persist_trust(Some(group.id));
        inner.store.ensure(group.id);
        Ok(group.id)
    }

    /// The group's members.
    pub async fn list_members(&self, group_id: Uuid) -> Result<Vec<Membership>, CoreError> {
        Ok(self.inner.lock().await.connect.list_members(group_id).await?)
    }

    /// Removes a member; the holders rotate at their next sync.
    pub async fn remove_member(&self, group_id: Uuid, user_id: Uuid) -> Result<(), CoreError> {
        self.inner.lock().await.connect.remove_member(group_id, user_id).await?;
        Ok(())
    }

    /// Changes a member's role.
    pub async fn set_member_role(&self, group_id: Uuid, user_id: Uuid, role: String) -> Result<(), CoreError> {
        self.inner.lock().await.connect.set_member_role(group_id, user_id, &role).await?;
        Ok(())
    }

    /// Leaves a group and forgets it.
    pub async fn leave_group(&self, group_id: Uuid) -> Result<(), CoreError> {
        let mut inner = self.inner.lock().await;
        inner.connect.leave_group(group_id).await?;
        inner.store.forget_group(group_id);
        inner.keyring.forget_group(group_id);
        for key in inner.storage.keys(format!("k/{group_id}/")) {
            inner.storage.delete(key);
        }
        Ok(())
    }

    /// An invite link (`/join/<code>#<secret>`, to be prefixed with the
    /// origin): the invite on the server, its secret in the group's
    /// objects for every holder to verify joiners with.
    pub async fn create_invite(&self, group_id: Uuid, role: String, expires_at_ms: i64, max_uses: i32) -> Result<String, CoreError> {
        let mut inner = self.inner.lock().await;
        let user = inner.user()?;
        let invite = inner.connect.create_invite(group_id, &role, expires_at_ms, max_uses).await?;
        let secret = crate::invite::InviteSecret::mint(&mut inner.rng);
        let now = inner.now();
        let doc = Document::new(user, now, InviteSecret { invite_id: invite.id, secret: secret.bytes().to_vec() });
        let id = ids::random(&mut inner.rng);
        inner.syncer().create(group_id, id, ObjectType::InviteSecret, doc.encode())?;
        Ok(join_link(&invite.code, &secret))
    }

    /// Joins through a link: accepts the invite and asks the holders for
    /// the keys with the invite's tag; a holder approves at its next look.
    pub async fn join(&self, link: String) -> Result<JoinResult, CoreError> {
        let mut inner = self.inner.lock().await;
        let (code, secret) = parse_join_link(&link).ok_or(CoreError::Link)?;
        let (membership, invite) = inner.connect.accept_invite(&code).await?;
        inner.store.ensure(membership.group_id);
        let inner = &mut *inner;
        let mut kx = KeyExchange { device_id: inner.device_id, device_key: &*inner.device_key, keyring: &mut inner.keyring, pins: &mut inner.pins, rng: &mut inner.rng };
        let request = kx.request_keys(&inner.connect, membership.group_id, Some((invite.id, &secret))).await?;
        Ok(JoinResult { group_id: membership.group_id, request_id: request.id })
    }

    /// The key requests this device may approve.
    pub async fn pending_key_requests(&self) -> Result<Vec<PendingRequestView>, CoreError> {
        let inner = self.inner.lock().await;
        let secrets = inner.invite_secrets();
        let pending = crate::keys::pending_requests_for(inner.device_id, &inner.connect, |invite_id| secrets.iter().find(|(id, _)| *id == invite_id).map(|(_, s)| s.clone())).await?;
        Ok(pending
            .into_iter()
            .map(|p| PendingRequestView {
                request_id: p.request.id,
                group_id: p.request.group_id,
                device_id: p.request.device.id,
                user_id: p.request.device.user_id,
                platform: p.request.device.platform,
                fingerprint: p.fingerprint,
                invite_verified: p.invite_verified,
                created_at_ms: p.request.created_at_ms,
            })
            .collect())
    }

    /// Approves a request: the person compared the fingerprint, or the
    /// invite vouched for it.
    pub async fn approve_key_request(&self, request_id: Uuid) -> Result<(), CoreError> {
        let mut inner = self.inner.lock().await;
        let inner = &mut *inner;
        let requests = crate::keys::Devices::list_key_requests(&inner.connect, inner.device_id).await?;
        let request = requests.into_iter().find(|r| r.id == request_id).ok_or(CoreError::Api { code: "not_found".into(), message: "the request is gone".into() })?;
        let mut kx = KeyExchange { device_id: inner.device_id, device_key: &*inner.device_key, keyring: &mut inner.keyring, pins: &mut inner.pins, rng: &mut inner.rng };
        kx.approve(&inner.connect, &request).await?;
        inner.persist_trust(Some(request.group_id));
        Ok(())
    }

    /// Mints the next key generation for the group (after a removal).
    pub async fn rotate_keys(&self, group_id: Uuid) -> Result<(), CoreError> {
        let mut inner = self.inner.lock().await;
        let inner = &mut *inner;
        let mut kx = KeyExchange { device_id: inner.device_id, device_key: &*inner.device_key, keyring: &mut inner.keyring, pins: &mut inner.pins, rng: &mut inner.rng };
        kx.rotate(&inner.connect, group_id).await?;
        inner.persist_trust(Some(group_id));
        Ok(())
    }

    // ---- Sync ----

    /// `device` said a holder pushed history for the request.
    pub async fn history_ready(&self, group_id: Uuid, request_id: Uuid, through_seq: i64, complete: bool) {
        self.inner.lock().await.syncer().history_ready(group_id, request_id, through_seq, complete);
    }

    /// One sync of everything: keys received, every held group pulled,
    /// acked and pushed, history served, keys asked for where the user is
    /// a member and the device holds none.
    pub async fn sync(&self) -> Result<SyncReport, CoreError> {
        let mut inner = self.inner.lock().await;
        let inner = &mut *inner;
        let mut report = SyncReport::default();
        let groups = inner.connect.list_groups().await?;
        let mut kx = KeyExchange { device_id: inner.device_id, device_key: &*inner.device_key, keyring: &mut inner.keyring, pins: &mut inner.pins, rng: &mut inner.rng };
        for event in kx.receive(&inner.connect, None).await? {
            match event {
                KeyEvent::Received { group_id, .. } => {
                    if !report.keys_received.contains(&group_id) {
                        report.keys_received.push(group_id);
                    }
                }
                KeyEvent::Pin(PinMismatch { device_id, pinned, offered }) => report.pin_alerts.push(PinAlert { device_id, pinned: fingerprint(&pinned), offered: fingerprint(&offered) }),
                _ => {}
            }
        }
        inner.persist_trust(None);
        for g in groups {
            if !inner.keyring.holds(g.id) {
                let mut kx = KeyExchange { device_id: inner.device_id, device_key: &*inner.device_key, keyring: &mut inner.keyring, pins: &mut inner.pins, rng: &mut inner.rng };
                if let Err(e) = kx.request_keys(&inner.connect, g.id, None).await {
                    report.failed.push(format!("{}: keys: {e}", g.id));
                }
                continue;
            }
            inner.store.ensure(g.id);
            let mut syncer = Syncer { device_id: inner.device_id, store: &mut inner.store, keyring: &inner.keyring, rng: &mut inner.rng };
            match syncer.sync_group(&inner.connect, g.id).await {
                Ok(events) => {
                    let mut changed = false;
                    for e in events {
                        match e {
                            SyncEvent::Applied { .. } | SyncEvent::Accepted { .. } | SyncEvent::EditDropped { .. } | SyncEvent::BecameHolder => changed = true,
                            SyncEvent::Invalid { id, reason } => report.invalid.push(InvalidWrite { group_id: g.id, object_id: id, reason }),
                            SyncEvent::StartedOver => {
                                report.started_over.push(g.id);
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                    if changed {
                        report.changed.push(g.id);
                    }
                }
                Err(e) => report.failed.push(format!("{}: {e}", g.id)),
            }
            let mut syncer = Syncer { device_id: inner.device_id, store: &mut inner.store, keyring: &inner.keyring, rng: &mut inner.rng };
            if let Err(e) = syncer.serve_history(&inner.connect, g.id).await {
                report.failed.push(format!("{}: history: {e}", g.id));
            }
        }
        Ok(report)
    }

    // ---- Objects ----

    /// Every live object of a group this device can read.
    pub async fn list_objects(&self, group_id: Uuid) -> Vec<ObjectView> {
        let inner = self.inner.lock().await;
        let Some(group) = inner.store.group(group_id) else { return Vec::new() };
        group.rows.keys().filter_map(|id| inner.view(group_id, *id)).collect()
    }

    /// One object, if held and readable.
    pub async fn read_object(&self, group_id: Uuid, id: Uuid) -> Option<ObjectView> {
        self.inner.lock().await.view(group_id, id)
    }

    /// A new object by the signed-in user, pushed at the next sync. A
    /// per-user singleton (a reaction, a vote) and a group singleton (the
    /// profile) get their derived id.
    pub async fn create_object(&self, group_id: Uuid, parent_id: Option<Uuid>, body: ObjectBody) -> Result<Uuid, CoreError> {
        let mut inner = self.inner.lock().await;
        let user = inner.user()?;
        let object_type = body.object_type();
        let id = match (object_type, parent_id) {
            (ObjectType::GroupProfile, _) => ids::group_profile(group_id),
            (ObjectType::MemberProfile, _) => ids::member_profile(group_id, user),
            (ObjectType::Reaction, Some(parent)) => ids::reaction(parent, user),
            (ObjectType::Vote, Some(parent)) => ids::vote(parent, user),
            _ => ids::random(&mut inner.rng),
        };
        self.create_with_id(&mut inner, group_id, id, parent_id, body)?;
        Ok(id)
    }

    /// A new object with a chosen id (the general expense pool,
    /// `general_pool_id`).
    pub async fn create_object_with_id(&self, group_id: Uuid, id: Uuid, parent_id: Option<Uuid>, body: ObjectBody) -> Result<(), CoreError> {
        let mut inner = self.inner.lock().await;
        self.create_with_id(&mut inner, group_id, id, parent_id, body)
    }

    /// Replaces an object's body; the common fields stay, the edit time
    /// moves.
    pub async fn edit_object(&self, group_id: Uuid, id: Uuid, body: ObjectBody) -> Result<(), CoreError> {
        let mut inner = self.inner.lock().await;
        let current = inner.view(group_id, id).ok_or(CoreError::Unknown { id })?;
        if current.body.object_type() != body.object_type() {
            return Err(CoreError::Invalid { reason: "an object keeps its type".into() });
        }
        body.validate().map_err(|e| CoreError::Invalid { reason: e.to_string() })?;
        let now = inner.now();
        let meta = Meta { parent_id: current.parent_id, author: current.author, authored_at: current.authored_at_ms, edited_at: now.max(current.authored_at_ms) };
        let mut fields = meta.to_fields();
        fields.extend(body.body_fields());
        inner.syncer().edit(group_id, id, crate::wire::encode(&fields))?;
        Ok(())
    }

    /// Deletes an object and the children this device holds.
    pub async fn delete_object(&self, group_id: Uuid, id: Uuid) -> Result<(), CoreError> {
        Ok(self.inner.lock().await.syncer().delete(group_id, id)?)
    }

    /// The group profile's id.
    pub fn group_profile_id(&self, group_id: Uuid) -> Uuid {
        ids::group_profile(group_id)
    }

    /// A member profile's id.
    pub fn member_profile_id(&self, group_id: Uuid, user_id: Uuid) -> Uuid {
        ids::member_profile(group_id, user_id)
    }

    /// The general expense pool's id.
    pub fn general_pool_id(&self, group_id: Uuid) -> Uuid {
        ids::general_pool(group_id)
    }

    // ---- Notes ----

    /// A note's body: its `note_update` children combined.
    pub async fn read_note(&self, group_id: Uuid, note_id: Uuid) -> Result<String, CoreError> {
        Ok(self.inner.lock().await.syncer().open_note(group_id, note_id)?.text())
    }

    /// Ends an editing session with the whole text: one `note_update`
    /// child when something changed. Returns whether one was written.
    pub async fn save_note(&self, group_id: Uuid, note_id: Uuid, text: String) -> Result<bool, CoreError> {
        let mut inner = self.inner.lock().await;
        let user = inner.user()?;
        let now = inner.now();
        let mut doc = inner.syncer().open_note(group_id, note_id)?;
        doc.set_text(&text);
        Ok(inner.syncer().save_note_session(group_id, note_id, &mut doc, user, now)?.is_some())
    }

    // ---- Files ----

    /// Encrypts and uploads a file into an object's slot; the reference
    /// goes into the object's value (the shell edits the object with it).
    pub async fn upload_file(&self, group_id: Uuid, object_id: Uuid, slot: String, bytes: Vec<u8>) -> Result<FileRef, CoreError> {
        let mut inner = self.inner.lock().await;
        let content_key = inner.rng.bytes32();
        let nonce = inner.rng.nonce24();
        let mut stored = nonce.to_vec();
        stored.extend_from_slice(&xchacha_seal(&content_key, &nonce, &blob_aad(group_id, object_id, &slot), &bytes));
        let key = blob_key(group_id, object_id, &slot);
        let grant = inner.connect.presign_upload(group_id, &key, stored.len() as i64).await?;
        let response = inner
            .connect
            .transport()
            .send(Request { method: "PUT".to_owned(), url: grant.url, headers: vec![Header::new("content-type", "application/octet-stream")], body: stored.clone() })
            .await
            .map_err(|e| CoreError::Api { code: "unavailable".into(), message: e.to_string() })?;
        if !(200..300).contains(&response.status) {
            return Err(CoreError::Api { code: "unavailable".into(), message: format!("upload: HTTP {}", response.status) });
        }
        Ok(FileRef { slot, sha256: sha256(&stored).to_vec(), size: stored.len() as i64, content_key: content_key.to_vec() })
    }

    /// Downloads and decrypts a file of an object.
    pub async fn download_file(&self, group_id: Uuid, object_id: Uuid, file: FileRef) -> Result<Vec<u8>, CoreError> {
        let inner = self.inner.lock().await;
        let key = blob_key(group_id, object_id, &file.slot);
        let grant = inner.connect.presign_download(group_id, &key).await?;
        let response = inner
            .connect
            .transport()
            .send(Request { method: "GET".to_owned(), url: grant.url, headers: Vec::new(), body: Vec::new() })
            .await
            .map_err(|e| CoreError::Api { code: "unavailable".into(), message: e.to_string() })?;
        if response.status != 200 {
            return Err(CoreError::Api { code: "not_found".into(), message: format!("download: HTTP {}", response.status) });
        }
        if sha256(&response.body).as_slice() != file.sha256.as_slice() {
            return Err(CoreError::Crypto { message: "the file's bytes are not the ones the object names".into() });
        }
        let content_key: [u8; 32] = file.content_key.as_slice().try_into().map_err(|_| CoreError::Crypto { message: "a file key is 32 bytes".into() })?;
        if response.body.len() < 24 {
            return Err(CoreError::Crypto { message: "the file is too short to be one".into() });
        }
        let (nonce, ciphertext) = response.body.split_at(24);
        let nonce: [u8; 24] = nonce.try_into().expect("24 bytes");
        Ok(xchacha_open(&content_key, &nonce, &blob_aad(group_id, object_id, &file.slot), ciphertext)?)
    }
}

impl Core {
    fn create_with_id(&self, inner: &mut Inner, group_id: Uuid, id: Uuid, parent_id: Option<Uuid>, body: ObjectBody) -> Result<(), CoreError> {
        let user = inner.user()?;
        let object_type = body.object_type();
        body.validate().map_err(|e| CoreError::Invalid { reason: e.to_string() })?;
        let now = inner.now();
        let meta = Meta { parent_id, author: user, authored_at: now, edited_at: now };
        match (object_type.parent(), parent_id) {
            (crate::schema::ParentRule::Never, Some(_)) => return Err(CoreError::Invalid { reason: "this object cannot have a parent".into() }),
            (crate::schema::ParentRule::Required, None) => return Err(CoreError::Invalid { reason: "this object needs a parent".into() }),
            _ => {}
        }
        let mut fields = meta.to_fields();
        fields.extend(body.body_fields());
        inner.store.ensure(group_id);
        inner.syncer().create(group_id, id, object_type, crate::wire::encode(&fields))?;
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{ModelServer, block_on};
    use crate::shell::{FixedClock, MemorySecrets, MemoryStorage, SeededRandom, SoftwareDeviceKey};

    /// A phone: a core over the in-memory server, signed in as `user`
    /// through a kept session token, its device registered.
    fn phone(server: &Arc<ModelServer>, n: u8, user: Uuid) -> Arc<Core> {
        let token = format!("tok-{n}");
        server.session(&token, user);
        let secrets = Arc::new(MemorySecrets::new());
        secrets.set_secret(SECRET_SESSION_TOKEN.to_owned(), token);
        let mut rng = CoreRng::from_seed([n; 32]);
        let key = Arc::new(SoftwareDeviceKey::generate(&mut rng));
        let transport: Arc<dyn Transport> = server.clone();
        let core = Core::new(transport, Arc::new(MemoryStorage::new()), secrets, key, Arc::new(SeededRandom::new([n; 32])), Arc::new(FixedClock::at(1_000 + i64::from(n))), "https://app.example".to_owned());
        let identity = block_on(core.restore_session()).unwrap().expect("the kept token signs in");
        assert_eq!(identity.id, user.to_string());
        block_on(core.register_device(if n.is_multiple_of(2) { "ios".to_owned() } else { "android".to_owned() })).unwrap();
        core
    }

    #[test]
    fn two_phones_share_a_group_through_the_whole_facade() {
        let server = Arc::new(ModelServer::new("https://app.example", [1; 32]));
        let (ua, ub) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let a = phone(&server, 1, ua);
        let b = phone(&server, 2, ub);
        assert_eq!(block_on(a.user_id()), Some(ua));

        // A makes the group, its profile and an invite link; B joins.
        let g = block_on(a.create_group(None)).unwrap();
        let profile = block_on(a.create_object(g, None, ObjectBody::GroupProfile(GroupProfile { name: "Us".into(), ..Default::default() }))).unwrap();
        assert_eq!(profile, a.group_profile_id(g));
        let report = block_on(a.sync()).unwrap();
        assert!(report.changed.contains(&g), "{report:?}");
        assert!(report.failed.is_empty(), "{report:?}");
        let link = block_on(a.create_invite(g, "member".into(), 0, 0)).unwrap();
        assert!(link.starts_with("/join/"), "{link}");
        block_on(a.sync()).unwrap();
        let joined = block_on(b.join(link)).unwrap();
        assert_eq!(joined.group_id, g);
        assert!(!block_on(b.list_groups()).unwrap()[0].holds_keys);

        // A sees the request with the invite vouching for it, approves.
        let pending = block_on(a.pending_key_requests()).unwrap();
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(pending[0].invite_verified, Some(true));
        assert_eq!(pending[0].fingerprint, block_on(b.fingerprint()));
        assert_eq!(pending[0].user_id, ub);
        block_on(a.approve_key_request(pending[0].request_id)).unwrap();
        let report = block_on(b.sync()).unwrap();
        assert_eq!(report.keys_received, vec![g], "{report:?}");
        assert!(report.changed.contains(&g));
        assert!(block_on(b.list_groups()).unwrap()[0].holds_keys);
        let objects = block_on(b.list_objects(g));
        assert!(objects.iter().any(|o| matches!(&o.body, ObjectBody::GroupProfile(p) if p.name == "Us")), "{objects:?}");

        // B posts; A sees it; A edits it; B sees the edit.
        let post = block_on(b.create_object(g, None, ObjectBody::Post(Post { text: "hello".into() }))).unwrap();
        assert!(block_on(b.read_object(g, post)).unwrap().pending);
        block_on(b.sync()).unwrap();
        block_on(a.sync()).unwrap();
        let seen = block_on(a.read_object(g, post)).unwrap();
        assert_eq!((seen.author, seen.pending, seen.seq > 0), (ub, false, true));
        block_on(a.edit_object(g, post, ObjectBody::Post(Post { text: "hello!".into() }))).unwrap();
        block_on(a.sync()).unwrap();
        block_on(b.sync()).unwrap();
        assert!(matches!(block_on(b.read_object(g, post)).unwrap().body, ObjectBody::Post(Post { ref text }) if text == "hello!"));
        assert!(matches!(block_on(a.create_object(g, None, ObjectBody::Comment(Comment { text: "x".into() }))), Err(CoreError::Invalid { .. })), "a comment needs a parent");

        // Notes.
        let note = block_on(a.create_object(g, None, ObjectBody::Note(Note { title: "List".into() }))).unwrap();
        assert!(block_on(a.save_note(g, note, "milk\n".into())).unwrap());
        assert!(!block_on(a.save_note(g, note, "milk\n".into())).unwrap(), "nothing new");
        block_on(a.sync()).unwrap();
        block_on(b.sync()).unwrap();
        assert_eq!(block_on(b.read_note(g, note)).unwrap(), "milk\n");

        // Files: a photo under the post, its bytes in the bucket.
        let photo = block_on(b.create_object(g, Some(post), ObjectBody::Photo(Photo { width: 1, height: 1, ..Default::default() }))).unwrap();
        let file = block_on(b.upload_file(g, photo, "full".into(), vec![7; 1000])).unwrap();
        assert!(server.bucket_has(&blob_key(g, photo, "full")));
        assert_eq!(file.size, 1000 + 24 + 16);
        block_on(b.edit_object(g, photo, ObjectBody::Photo(Photo { width: 1, height: 1, full: Some(file.clone()), ..Default::default() }))).unwrap();
        block_on(b.sync()).unwrap();
        block_on(a.sync()).unwrap();
        let ObjectBody::Photo(p) = block_on(a.read_object(g, photo)).unwrap().body else { panic!("a photo") };
        assert_eq!(block_on(a.download_file(g, photo, p.full.unwrap())).unwrap(), vec![7; 1000]);

        // Deleting the post takes the photo with it.
        block_on(b.delete_object(g, post)).unwrap();
        block_on(b.sync()).unwrap();
        block_on(a.sync()).unwrap();
        assert!(block_on(a.read_object(g, post)).is_none());
        assert!(block_on(a.read_object(g, photo)).is_none());

        // A's second phone: a member without keys asks, A approves as its
        // own, and it catches up on everything.
        let a2 = phone(&server, 3, ua);
        let report = block_on(a2.sync()).unwrap();
        assert!(report.keys_received.is_empty() && report.failed.is_empty(), "{report:?}");
        let pending = block_on(a.pending_key_requests()).unwrap();
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(pending[0].invite_verified, None, "no invite: the fingerprint decides");
        block_on(a.approve_key_request(pending[0].request_id)).unwrap();
        let report = block_on(a2.sync()).unwrap();
        assert_eq!(report.keys_received, vec![g]);
        assert_eq!(block_on(a2.read_note(g, note)).unwrap(), "milk\n");

        // B is removed; A rotates; what A writes after is not B's to read.
        block_on(a.remove_member(g, ub)).unwrap();
        block_on(a.rotate_keys(g)).unwrap();
        let after = block_on(a.create_object(g, None, ObjectBody::Post(Post { text: "after".into() }))).unwrap();
        block_on(a.sync()).unwrap();
        let report = block_on(b.sync()).unwrap();
        assert!(report.changed.is_empty(), "B is in no group now: {report:?}");
        assert!(block_on(b.read_object(g, after)).is_none());
        let report = block_on(a2.sync()).unwrap();
        assert_eq!(report.keys_received, vec![g], "the rotation reached A's other phone: {report:?}");
        assert!(block_on(a2.read_object(g, after)).is_some());

        // A reopened core over the same storage and secrets holds the same.
        let storage = Arc::new(MemoryStorage::new());
        let secrets = Arc::new(MemorySecrets::new());
        secrets.set_secret(SECRET_SESSION_TOKEN.to_owned(), "tok-9".to_owned());
        server.session("tok-9", ua);
        let mut rng = CoreRng::from_seed([9; 32]);
        let key = Arc::new(SoftwareDeviceKey::generate(&mut rng));
        let transport: Arc<dyn Transport> = server.clone();
        let fresh = Core::new(transport.clone(), storage.clone(), secrets.clone(), key.clone(), Arc::new(SeededRandom::new([9; 32])), Arc::new(FixedClock::at(5_000)), "https://app.example".to_owned());
        block_on(fresh.restore_session()).unwrap();
        block_on(fresh.register_device("web".to_owned())).unwrap();
        block_on(fresh.sync()).unwrap();
        let pending = block_on(a.pending_key_requests()).unwrap();
        block_on(a.approve_key_request(pending[0].request_id)).unwrap();
        block_on(fresh.sync()).unwrap();
        let device_id = block_on(fresh.device_id());
        drop(fresh);
        let again = Core::new(transport, storage, secrets, key, Arc::new(SeededRandom::new([9; 32])), Arc::new(FixedClock::at(6_000)), "https://app.example".to_owned());
        assert_eq!(block_on(again.device_id()), device_id, "the device id is kept");
        assert!(block_on(again.list_objects(g)).iter().any(|o| o.id == after), "the store and the keys are kept");
    }
}
