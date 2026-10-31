//! The core for the browser (`docs/01-deappje/01-architecture.md`, "The
//! shells"): wasm-bindgen exports of [`Core`](crate::facade::Core) and, for
//! the web client's tests, of [`ModelServer`] and
//! [`SoftwareDeviceKey`](crate::shell::SoftwareDeviceKey). The shell hands
//! the seams in as plain JavaScript objects with the methods of `Random`,
//! `Clock`, `Secrets`, `DeviceKey`, `Storage` and `Transport` (its `send`
//! returns a Promise); this module wraps each in a type implementing the
//! core's trait. Every `Core` method is a method returning a Promise; what
//! crosses it crosses as plain objects, with the core's field names
//! (`TYPES` is their TypeScript, which wasm-bindgen writes into the glue's
//! `.d.ts`), an id as its text and a failure as an `Error` whose `kind`
//! names the [`CoreError`] variant.
//!
//! Only on wasm32: on the host the phones' UniFFI exports are the bindings.

#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::connect::Membership;
use crate::facade::{Core, CoreError, GroupView, JoinResult, PendingRequestView, SyncReport};
use crate::harness::ModelServer;
use crate::kratos::{Identity, LoginStart};
use crate::shell::{Clock, DeviceKey, Random, Secrets, SoftwareDeviceKey, Storage};
use crate::transport::{Header, Request, Response, Transport, TransportError};

/// The TypeScript of everything that crosses: the seams a shell provides
/// and the records the core answers with. Kept beside the Rust it mirrors;
/// a record's fields are the core's, in the core's names.
#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/** Entropy: 32 random bytes per call (`crypto.getRandomValues`). */
export interface RandomSeam {
    randomBytes(): Uint8Array;
}

/** The wall clock, milliseconds since the Unix epoch (`Date.now()`). */
export interface ClockSeam {
    nowMs(): number;
}

/** Small named strings: the device id and the session token. */
export interface SecretsSeam {
    getSecret(name: string): string | undefined;
    setSecret(name: string, value: string): void;
    deleteSecret(name: string): void;
}

/**
 * The device's X25519 key; the private half never reaches the core. Both
 * methods are synchronous, which is why a WebCrypto key cannot be one yet
 * (`crypto.subtle.deriveBits` only answers a Promise): `SoftwareDeviceKey`
 * is the implementation the browser has today.
 */
export interface DeviceKeySeam {
    publicKeyBytes(): Uint8Array;
    dhBytes(theirPublic: Uint8Array): Uint8Array;
}

/**
 * Named blobs the core writes through on every change and reads back
 * whole when it opens. The bytes handed to `put` are the shell's to keep.
 */
export interface StorageSeam {
    get(key: string): Uint8Array | undefined;
    put(key: string, value: Uint8Array): void;
    delete(key: string): void;
    keys(prefix: string): string[];
}

export interface TransportHeader {
    name: string;
    value: string;
}

/** One HTTP request as the core hands it over; `body` is empty for a GET. */
export interface TransportRequest {
    method: string;
    url: string;
    headers: TransportHeader[];
    body: Uint8Array;
}

/** What came back; header names lowercased. */
export interface TransportResponse {
    status: number;
    headers: TransportHeader[];
    body: Uint8Array;
}

/**
 * HTTP as the shell does it (`fetch` in a browser). A rejected Promise is
 * the network failing, never the server answering.
 */
export interface TransportSeam {
    send(request: TransportRequest): Promise<TransportResponse>;
}

/** The signed-in identity, as Kratos's whoami has it. */
export interface Identity {
    id: string;
    email?: string;
}

/** A native sign-in under way (the phones' flow; the browser signs in by redirect). */
export interface LoginStart {
    flow_id: string;
    browser_url: string;
    exchange_code: string;
}

/** A group as the device knows it. */
export interface GroupView {
    id: string;
    parent_group_id?: string;
    /** Without a key the device can read nothing of it. */
    holds_keys: boolean;
    /** Where the device stands in the group's log. */
    phase: "new" | "catching_up" | "holder";
}

export interface Membership {
    id: string;
    group_id: string;
    user_id: string;
    role: "admin" | "member" | "viewer";
    joined_at_ms: number;
}

/** What joining through a link did: the key request waits for a holder's approval. */
export interface JoinResult {
    group_id: string;
    request_id: string;
}

/** A key request as the approving screen shows it. */
export interface PendingRequestView {
    request_id: string;
    group_id: string;
    device_id: string;
    user_id: string;
    platform: string;
    /** What the requesting device shows on its screen. */
    fingerprint: string;
    /** Whether the invite it came through vouches for it; absent when it came through none this device knows. */
    invite_verified?: boolean;
    created_at_ms: number;
}

export interface InvalidWrite {
    group_id: string;
    object_id: string;
    reason: string;
}

export interface PinAlert {
    device_id: string;
    pinned: string;
    offered: string;
}

/** What a sync did. */
export interface SyncReport {
    /** Groups whose rows changed: re-read them. */
    changed: string[];
    keys_received: string[];
    started_over: string[];
    invalid: InvalidWrite[];
    pin_alerts: PinAlert[];
    /** Groups where a sync failed, with why; the rest went on. */
    failed: string[];
}

/** A file in the bucket, inside its object's value. Byte fields cross as number arrays. */
export interface FileRef {
    slot: string;
    sha256: number[];
    size: number;
    content_key: number[];
}

export type ObjectTypeName =
    | "GroupProfile" | "MemberProfile" | "InviteSecret" | "Post" | "Photo" | "Album" | "Comment" | "Reaction"
    | "TodoList" | "TodoItem" | "Note" | "NoteUpdate" | "Expense" | "Settlement" | "TicketShare" | "ExpensePool"
    | "Event" | "Poll" | "Vote" | "Message" | "Venue";

/** The typed body of an object: the type's name and its fields, in the core's names. */
export type ObjectBody =
    | { type: "Post"; fields: { text: string } }
    | { type: "GroupProfile"; fields: { name: string; description: string; avatar: number[]; message_retention_months?: number; kind: "Group" | "Chat" } }
    | { type: "Comment"; fields: { text: string } }
    | { type: "Note"; fields: { title: string } }
    | { type: Exclude<ObjectTypeName, "Post" | "GroupProfile" | "Comment" | "Note">; fields: Record<string, unknown> };

/** An object as a screen shows it. */
export interface ObjectView {
    id: string;
    parent_id?: string;
    author: string;
    authored_at_ms: number;
    edited_at_ms: number;
    /** Its seq on the server; 0 while a create is not yet accepted. */
    seq: number;
    /** Whether a write of this device is not yet accepted. */
    pending: boolean;
    body: ObjectBody;
}

/** How a Core method fails: `kind` is the core's error variant, `code` the server's answer for an `api` one. */
export interface CoreError extends Error {
    kind: "api" | "not_signed_in" | "no_key" | "invalid" | "unknown" | "login" | "link" | "crypto" | "pin";
    code?: string;
}
"#;

#[wasm_bindgen]
extern "C" {
    /// A `RandomSeam`.
    #[wasm_bindgen(typescript_type = "RandomSeam")]
    pub type JsRandom;
    #[wasm_bindgen(method, js_name = randomBytes)]
    fn random_bytes(this: &JsRandom) -> Vec<u8>;

    /// A `ClockSeam`.
    #[wasm_bindgen(typescript_type = "ClockSeam")]
    pub type JsClock;
    #[wasm_bindgen(method, js_name = nowMs)]
    fn now_ms(this: &JsClock) -> f64;

    /// A `SecretsSeam`.
    #[wasm_bindgen(typescript_type = "SecretsSeam")]
    pub type JsSecrets;
    #[wasm_bindgen(method, js_name = getSecret)]
    fn get_secret(this: &JsSecrets, name: &str) -> Option<String>;
    #[wasm_bindgen(method, js_name = setSecret)]
    fn set_secret(this: &JsSecrets, name: &str, value: &str);
    #[wasm_bindgen(method, js_name = deleteSecret)]
    fn delete_secret(this: &JsSecrets, name: &str);

    /// A `DeviceKeySeam`.
    #[wasm_bindgen(typescript_type = "DeviceKeySeam")]
    pub type JsDeviceKey;
    #[wasm_bindgen(method, js_name = publicKeyBytes)]
    fn public_key_bytes(this: &JsDeviceKey) -> Vec<u8>;
    #[wasm_bindgen(method, js_name = dhBytes)]
    fn dh_bytes(this: &JsDeviceKey, their_public: Vec<u8>) -> Vec<u8>;

    /// A `StorageSeam`.
    #[wasm_bindgen(typescript_type = "StorageSeam")]
    pub type JsStorage;
    #[wasm_bindgen(method)]
    fn get(this: &JsStorage, key: &str) -> Option<Vec<u8>>;
    #[wasm_bindgen(method)]
    fn put(this: &JsStorage, key: &str, value: Vec<u8>);
    #[wasm_bindgen(method)]
    fn delete(this: &JsStorage, key: &str);
    #[wasm_bindgen(method)]
    fn keys(this: &JsStorage, prefix: &str) -> JsValue;

    /// A `TransportSeam`.
    #[wasm_bindgen(typescript_type = "TransportSeam")]
    pub type JsTransport;
    #[wasm_bindgen(method, catch)]
    fn send(this: &JsTransport, request: JsValue) -> Result<Promise, JsValue>;
}

impl Random for JsRandom {
    fn random_bytes(&self) -> Vec<u8> {
        JsRandom::random_bytes(self)
    }
}

impl Clock for JsClock {
    fn now_ms(&self) -> i64 {
        JsClock::now_ms(self) as i64
    }
}

impl Secrets for JsSecrets {
    fn get_secret(&self, name: String) -> Option<String> {
        JsSecrets::get_secret(self, &name)
    }

    fn set_secret(&self, name: String, value: String) {
        JsSecrets::set_secret(self, &name, &value)
    }

    fn delete_secret(&self, name: String) {
        JsSecrets::delete_secret(self, &name)
    }
}

impl DeviceKey for JsDeviceKey {
    fn public_key_bytes(&self) -> Vec<u8> {
        JsDeviceKey::public_key_bytes(self)
    }

    fn dh_bytes(&self, their_public: Vec<u8>) -> Vec<u8> {
        JsDeviceKey::dh_bytes(self, their_public)
    }
}

impl Storage for JsStorage {
    fn get(&self, key: String) -> Option<Vec<u8>> {
        JsStorage::get(self, &key)
    }

    fn put(&self, key: String, value: Vec<u8>) {
        JsStorage::put(self, &key, value)
    }

    fn delete(&self, key: String) {
        JsStorage::delete(self, &key)
    }

    fn keys(&self, prefix: String) -> Vec<String> {
        Array::from(&JsStorage::keys(self, &prefix)).iter().filter_map(|k| k.as_string()).collect()
    }
}

#[async_trait::async_trait(?Send)]
impl Transport for JsTransport {
    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        let promise = JsTransport::send(self, request_to_js(&request)).map_err(|e| TransportError::Network(describe(&e)))?;
        let value = JsFuture::from(promise).await.map_err(|e| TransportError::Network(describe(&e)))?;
        response_from_js(&value).map_err(TransportError::Network)
    }
}

// ---- Plain objects ----

/// A plain object under construction.
struct Obj(Object);

impl Obj {
    fn new() -> Obj {
        Obj(Object::new())
    }

    fn set(self, name: &str, value: impl Into<JsValue>) -> Obj {
        Reflect::set(&self.0, &JsValue::from_str(name), &value.into()).expect("a plain object takes any property");
        self
    }

    /// `undefined` for `None`, like a missing optional field.
    fn opt(self, name: &str, value: Option<impl Into<JsValue>>) -> Obj {
        match value {
            Some(v) => self.set(name, v),
            None => self,
        }
    }

    fn id(self, name: &str, id: Uuid) -> Obj {
        self.set(name, id.to_string())
    }

    fn opt_id(self, name: &str, id: Option<Uuid>) -> Obj {
        self.opt(name, id.map(|i| i.to_string()))
    }

    fn ms(self, name: &str, ms: i64) -> Obj {
        self.set(name, ms as f64)
    }

    fn ids(self, name: &str, ids: &[Uuid]) -> Obj {
        self.set(name, ids.iter().map(|i| JsValue::from_str(&i.to_string())).collect::<Array>())
    }

    fn strings(self, name: &str, values: &[String]) -> Obj {
        self.set(name, values.iter().map(|s| JsValue::from_str(s)).collect::<Array>())
    }

    fn done(self) -> JsValue {
        self.0.into()
    }
}

/// A property of a plain object the shell handed over.
fn prop(value: &JsValue, name: &str) -> Result<JsValue, String> {
    Reflect::get(value, &JsValue::from_str(name)).map_err(|_| format!("{name}: not an object"))
}

fn string_prop(value: &JsValue, name: &str) -> Result<String, String> {
    prop(value, name)?.as_string().ok_or_else(|| format!("{name}: not a string"))
}

/// What a JavaScript error says.
fn describe(e: &JsValue) -> String {
    if let Some(error) = e.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

fn headers_to_js(headers: &[Header]) -> Array {
    headers.iter().map(|h| Obj::new().set("name", h.name.as_str()).set("value", h.value.as_str()).done()).collect()
}

fn request_to_js(r: &Request) -> JsValue {
    Obj::new().set("method", r.method.as_str()).set("url", r.url.as_str()).set("headers", headers_to_js(&r.headers)).set("body", Uint8Array::from(r.body.as_slice())).done()
}

fn response_from_js(v: &JsValue) -> Result<Response, String> {
    let status = prop(v, "status")?.as_f64().ok_or("status: not a number")?;
    if !(0.0..=65535.0).contains(&status) || status.fract() != 0.0 {
        return Err("status: not a status code".to_owned());
    }
    let headers = Array::from(&prop(v, "headers")?)
        .iter()
        .map(|h| Ok(Header { name: string_prop(&h, "name")?.to_ascii_lowercase(), value: string_prop(&h, "value")? }))
        .collect::<Result<Vec<_>, String>>()?;
    let body = prop(v, "body")?;
    let body = body.dyn_ref::<Uint8Array>().ok_or("body: not a Uint8Array")?.to_vec();
    Ok(Response { status: status as u16, headers, body })
}

fn identity_to_js(i: &Identity) -> JsValue {
    Obj::new().set("id", i.id.as_str()).opt("email", i.email.as_deref()).done()
}

fn login_start_to_js(s: &LoginStart) -> JsValue {
    Obj::new().set("flow_id", s.flow_id.as_str()).set("browser_url", s.browser_url.as_str()).set("exchange_code", s.exchange_code.as_str()).done()
}

fn login_start_from_js(v: &JsValue) -> Result<LoginStart, JsValue> {
    let read = |name| string_prop(v, name).map_err(|m| invalid(&format!("login start: {m}")));
    Ok(LoginStart { flow_id: read("flow_id")?, browser_url: read("browser_url")?, exchange_code: read("exchange_code")? })
}

fn group_to_js(g: &GroupView) -> JsValue {
    Obj::new().id("id", g.id).opt_id("parent_group_id", g.parent_group_id).set("holds_keys", g.holds_keys).set("phase", g.phase.as_str()).done()
}

fn membership_to_js(m: &Membership) -> JsValue {
    Obj::new().id("id", m.id).id("group_id", m.group_id).id("user_id", m.user_id).set("role", m.role.as_str()).ms("joined_at_ms", m.joined_at_ms).done()
}

fn join_to_js(j: &JoinResult) -> JsValue {
    Obj::new().id("group_id", j.group_id).id("request_id", j.request_id).done()
}

fn pending_to_js(p: &PendingRequestView) -> JsValue {
    Obj::new()
        .id("request_id", p.request_id)
        .id("group_id", p.group_id)
        .id("device_id", p.device_id)
        .id("user_id", p.user_id)
        .set("platform", p.platform.as_str())
        .set("fingerprint", p.fingerprint.as_str())
        .opt("invite_verified", p.invite_verified)
        .ms("created_at_ms", p.created_at_ms)
        .done()
}

fn report_to_js(r: &SyncReport) -> JsValue {
    let invalid: Array = r.invalid.iter().map(|w| Obj::new().id("group_id", w.group_id).id("object_id", w.object_id).set("reason", w.reason.as_str()).done()).collect();
    let pins: Array = r.pin_alerts.iter().map(|p| Obj::new().id("device_id", p.device_id).set("pinned", p.pinned.as_str()).set("offered", p.offered.as_str()).done()).collect();
    Obj::new().ids("changed", &r.changed).ids("keys_received", &r.keys_received).ids("started_over", &r.started_over).set("invalid", invalid).set("pin_alerts", pins).strings("failed", &r.failed).done()
}

fn list<T>(items: &[T], one: impl Fn(&T) -> JsValue) -> JsValue {
    items.iter().map(one).collect::<Array>().into()
}

// ---- Errors ----

/// A failure as the shell catches it: an `Error` with the message, a
/// `kind` naming the variant and, for the server's refusals, its `code`.
fn js_error(e: CoreError) -> JsValue {
    let (kind, code) = match &e {
        CoreError::Api { code, .. } => ("api", Some(code.clone())),
        CoreError::NotSignedIn => ("not_signed_in", None),
        CoreError::NoKey { .. } => ("no_key", None),
        CoreError::Invalid { .. } => ("invalid", None),
        CoreError::Unknown { .. } => ("unknown", None),
        CoreError::Login { .. } => ("login", None),
        CoreError::Link => ("link", None),
        CoreError::Crypto { .. } => ("crypto", None),
        CoreError::Pin { .. } => ("pin", None),
    };
    let error = js_sys::Error::new(&e.to_string());
    error.set_name("CoreError");
    Reflect::set(&error, &JsValue::from_str("kind"), &JsValue::from_str(kind)).expect("an Error takes a property");
    if let Some(code) = code {
        Reflect::set(&error, &JsValue::from_str("code"), &JsValue::from_str(&code)).expect("an Error takes a property");
    }
    error.into()
}

fn invalid(reason: &str) -> JsValue {
    js_error(CoreError::Invalid { reason: reason.to_owned() })
}

/// An id from its text.
fn id(s: &str, what: &str) -> Result<Uuid, JsValue> {
    s.parse().map_err(|_| invalid(&format!("{what}: not an id")))
}

fn opt_id(s: Option<String>, what: &str) -> Result<Option<Uuid>, JsValue> {
    s.map(|s| id(&s, what)).transpose()
}

/// A value the core answers with, as a plain object through serde.
fn serialize<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(value).map_err(|e| invalid(&e.to_string()))
}

fn deserialize<T: serde::de::DeserializeOwned>(value: JsValue, what: &str) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|e| invalid(&format!("{what}: {e}")))
}

fn done() -> JsValue {
    JsValue::UNDEFINED
}

// ---- Core ----

/// The core as the web client holds it. Every method but the id
/// derivations returns a Promise; a failure rejects it with a `CoreError`.
#[wasm_bindgen(js_name = Core)]
pub struct WasmCore {
    inner: Arc<Core>,
}

#[wasm_bindgen(js_class = Core)]
impl WasmCore {
    /// Opens the core over the shell's seams and the deployment's origin
    /// (`window.location.origin`). The device id is minted on first use
    /// and kept in the secrets; a kept session token is sent until it is
    /// refused.
    #[wasm_bindgen(constructor)]
    pub fn new(transport: JsTransport, storage: JsStorage, secrets: JsSecrets, device_key: JsDeviceKey, random: JsRandom, clock: JsClock, base_url: String) -> WasmCore {
        let transport: Arc<dyn Transport> = Arc::new(transport);
        let storage: Arc<dyn Storage> = Arc::new(storage);
        let secrets: Arc<dyn Secrets> = Arc::new(secrets);
        let device_key: Arc<dyn DeviceKey> = Arc::new(device_key);
        let random: Arc<dyn Random> = Arc::new(random);
        let clock: Arc<dyn Clock> = Arc::new(clock);
        WasmCore { inner: Core::new(transport, storage, secrets, device_key, random, clock, base_url) }
    }

    /// This device's id.
    #[wasm_bindgen(js_name = deviceId, unchecked_return_type = "string")]
    pub async fn device_id(&self) -> JsValue {
        JsValue::from_str(&self.inner.device_id().await.to_string())
    }

    /// This device's fingerprint, what the approver reads off its screen.
    #[wasm_bindgen(unchecked_return_type = "string")]
    pub async fn fingerprint(&self) -> JsValue {
        JsValue::from_str(&self.inner.fingerprint().await)
    }

    /// The signed-in user, once known.
    #[wasm_bindgen(js_name = userId, unchecked_return_type = "string | undefined")]
    pub async fn user_id(&self) -> JsValue {
        self.inner.user_id().await.map_or(JsValue::UNDEFINED, |u| JsValue::from_str(&u.to_string()))
    }

    // ---- Signing in ----

    /// Keeps a session token, or forgets it with `undefined`. The browser's
    /// session is a cookie: the web shell keeps a marker its transport
    /// recognises, so the core asks whoami and the cookie answers.
    #[wasm_bindgen(js_name = setSessionToken)]
    pub async fn set_session_token(&self, token: Option<String>) {
        self.inner.set_session_token(token).await
    }

    /// Who the kept session signs in as, if it still does.
    #[wasm_bindgen(js_name = restoreSession, unchecked_return_type = "Identity | undefined")]
    pub async fn restore_session(&self) -> Result<JsValue, JsValue> {
        Ok(self.inner.restore_session().await.map_err(js_error)?.map_or(JsValue::UNDEFINED, |i| identity_to_js(&i)))
    }

    /// Starts a native sign-in that returns to `returnTo` (the phones'
    /// flow; the browser is sent to Kratos's `/self-service/login/browser`).
    #[wasm_bindgen(js_name = startLogin, unchecked_return_type = "LoginStart")]
    pub async fn start_login(&self, return_to: String) -> Result<JsValue, JsValue> {
        Ok(login_start_to_js(&self.inner.start_login(return_to).await.map_err(js_error)?))
    }

    /// The browser came back with `code`: the session is kept and the
    /// identity known.
    #[wasm_bindgen(js_name = finishLogin, unchecked_return_type = "Identity")]
    pub async fn finish_login(&self, #[wasm_bindgen(unchecked_param_type = "LoginStart")] start: JsValue, code: String) -> Result<JsValue, JsValue> {
        let start = login_start_from_js(&start)?;
        Ok(identity_to_js(&self.inner.finish_login(start, code).await.map_err(js_error)?))
    }

    /// Ends the session.
    #[wasm_bindgen(unchecked_return_type = "undefined")]
    pub async fn logout(&self) -> Result<JsValue, JsValue> {
        self.inner.logout().await.map_err(js_error)?;
        Ok(done())
    }

    /// Registers this device with its public key (`web`). A repeat refreshes.
    #[wasm_bindgen(js_name = registerDevice, unchecked_return_type = "undefined")]
    pub async fn register_device(&self, platform: String) -> Result<JsValue, JsValue> {
        self.inner.register_device(platform).await.map_err(js_error)?;
        Ok(done())
    }

    /// Hands the server the push token (`apns` or `fcm`); empty clears.
    #[wasm_bindgen(js_name = setPushToken, unchecked_return_type = "undefined")]
    pub async fn set_push_token(&self, provider: String, token: String) -> Result<JsValue, JsValue> {
        self.inner.set_push_token(provider, token).await.map_err(js_error)?;
        Ok(done())
    }

    // ---- Groups ----

    /// The groups the user is in, with whether this device can read them.
    #[wasm_bindgen(js_name = listGroups, unchecked_return_type = "GroupView[]")]
    pub async fn list_groups(&self) -> Result<JsValue, JsValue> {
        Ok(list(&self.inner.list_groups().await.map_err(js_error)?, group_to_js))
    }

    /// A new group, or a subgroup of `parentGroupId`: created on the
    /// server, its first key minted here. The profile is the shell's first
    /// object in it. Answers the group's id.
    #[wasm_bindgen(js_name = createGroup, unchecked_return_type = "string")]
    pub async fn create_group(&self, parent_group_id: Option<String>) -> Result<JsValue, JsValue> {
        let parent = opt_id(parent_group_id, "parentGroupId")?;
        Ok(JsValue::from_str(&self.inner.create_group(parent).await.map_err(js_error)?.to_string()))
    }

    /// The group's members.
    #[wasm_bindgen(js_name = listMembers, unchecked_return_type = "Membership[]")]
    pub async fn list_members(&self, group_id: String) -> Result<JsValue, JsValue> {
        Ok(list(&self.inner.list_members(id(&group_id, "groupId")?).await.map_err(js_error)?, membership_to_js))
    }

    /// Removes a member; the holders rotate at their next sync.
    #[wasm_bindgen(js_name = removeMember, unchecked_return_type = "undefined")]
    pub async fn remove_member(&self, group_id: String, user_id: String) -> Result<JsValue, JsValue> {
        self.inner.remove_member(id(&group_id, "groupId")?, id(&user_id, "userId")?).await.map_err(js_error)?;
        Ok(done())
    }

    /// Changes a member's role (`admin`, `member`, `viewer`).
    #[wasm_bindgen(js_name = setMemberRole, unchecked_return_type = "undefined")]
    pub async fn set_member_role(&self, group_id: String, user_id: String, role: String) -> Result<JsValue, JsValue> {
        self.inner.set_member_role(id(&group_id, "groupId")?, id(&user_id, "userId")?, role).await.map_err(js_error)?;
        Ok(done())
    }

    /// Leaves a group and forgets it.
    #[wasm_bindgen(js_name = leaveGroup, unchecked_return_type = "undefined")]
    pub async fn leave_group(&self, group_id: String) -> Result<JsValue, JsValue> {
        self.inner.leave_group(id(&group_id, "groupId")?).await.map_err(js_error)?;
        Ok(done())
    }

    /// An invite link (`/join/<code>#<secret>`, to be prefixed with the
    /// origin). `expiresAtMs` 0 never expires, `maxUses` 0 is unbounded.
    #[wasm_bindgen(js_name = createInvite, unchecked_return_type = "string")]
    pub async fn create_invite(&self, group_id: String, role: String, expires_at_ms: f64, max_uses: i32) -> Result<JsValue, JsValue> {
        Ok(JsValue::from_str(&self.inner.create_invite(id(&group_id, "groupId")?, role, expires_at_ms as i64, max_uses).await.map_err(js_error)?))
    }

    /// Joins through a link, with or without its origin: accepts the invite
    /// and asks the holders for the keys with the invite's tag.
    #[wasm_bindgen(unchecked_return_type = "JoinResult")]
    pub async fn join(&self, link: String) -> Result<JsValue, JsValue> {
        Ok(join_to_js(&self.inner.join(link).await.map_err(js_error)?))
    }

    /// The key requests this device may approve.
    #[wasm_bindgen(js_name = pendingKeyRequests, unchecked_return_type = "PendingRequestView[]")]
    pub async fn pending_key_requests(&self) -> Result<JsValue, JsValue> {
        Ok(list(&self.inner.pending_key_requests().await.map_err(js_error)?, pending_to_js))
    }

    /// Approves a request: the person compared the fingerprint, or the
    /// invite vouched for it.
    #[wasm_bindgen(js_name = approveKeyRequest, unchecked_return_type = "undefined")]
    pub async fn approve_key_request(&self, request_id: String) -> Result<JsValue, JsValue> {
        self.inner.approve_key_request(id(&request_id, "requestId")?).await.map_err(js_error)?;
        Ok(done())
    }

    /// Mints the next key generation for the group (after a removal).
    #[wasm_bindgen(js_name = rotateKeys, unchecked_return_type = "undefined")]
    pub async fn rotate_keys(&self, group_id: String) -> Result<JsValue, JsValue> {
        self.inner.rotate_keys(id(&group_id, "groupId")?).await.map_err(js_error)?;
        Ok(done())
    }

    // ---- Sync ----

    /// The `history_ready` wake-up's payload; call `sync` after it.
    #[wasm_bindgen(js_name = historyReady, unchecked_return_type = "undefined")]
    pub async fn history_ready(&self, request_id: String, through_seq: f64, complete: bool) -> Result<JsValue, JsValue> {
        self.inner.history_ready(id(&request_id, "requestId")?, through_seq as i64, complete).await;
        Ok(done())
    }

    /// One sync of everything: keys received, every held group pulled,
    /// acked and pushed, history served, keys asked for where the user is
    /// a member and the device holds none.
    #[wasm_bindgen(unchecked_return_type = "SyncReport")]
    pub async fn sync(&self) -> Result<JsValue, JsValue> {
        Ok(report_to_js(&self.inner.sync().await.map_err(js_error)?))
    }

    // ---- Objects ----

    /// Every live object of a group this device can read, in id order.
    #[wasm_bindgen(js_name = listObjects, unchecked_return_type = "ObjectView[]")]
    pub async fn list_objects(&self, group_id: String) -> Result<JsValue, JsValue> {
        serialize(&self.inner.list_objects(id(&group_id, "groupId")?).await)
    }

    /// One object, if held and readable.
    #[wasm_bindgen(js_name = readObject, unchecked_return_type = "ObjectView | undefined")]
    pub async fn read_object(&self, group_id: String, object_id: String) -> Result<JsValue, JsValue> {
        match self.inner.read_object(id(&group_id, "groupId")?, id(&object_id, "objectId")?).await {
            Some(view) => serialize(&view),
            None => Ok(JsValue::UNDEFINED),
        }
    }

    /// A new object by the signed-in user, pushed at the next sync; answers
    /// its id. A per-user singleton (a reaction, a vote) and a group
    /// singleton (the profile) get their derived id.
    #[wasm_bindgen(js_name = createObject, unchecked_return_type = "string")]
    pub async fn create_object(&self, group_id: String, parent_id: Option<String>, #[wasm_bindgen(unchecked_param_type = "ObjectBody")] body: JsValue) -> Result<JsValue, JsValue> {
        let body = deserialize(body, "body")?;
        let created = self.inner.create_object(id(&group_id, "groupId")?, opt_id(parent_id, "parentId")?, body).await.map_err(js_error)?;
        Ok(JsValue::from_str(&created.to_string()))
    }

    /// A new object with a chosen id (the general expense pool).
    #[wasm_bindgen(js_name = createObjectWithId, unchecked_return_type = "undefined")]
    pub async fn create_object_with_id(&self, group_id: String, object_id: String, parent_id: Option<String>, #[wasm_bindgen(unchecked_param_type = "ObjectBody")] body: JsValue) -> Result<JsValue, JsValue> {
        let body = deserialize(body, "body")?;
        self.inner.create_object_with_id(id(&group_id, "groupId")?, id(&object_id, "objectId")?, opt_id(parent_id, "parentId")?, body).await.map_err(js_error)?;
        Ok(done())
    }

    /// Replaces an object's body; the common fields stay, the edit time
    /// moves.
    #[wasm_bindgen(js_name = editObject, unchecked_return_type = "undefined")]
    pub async fn edit_object(&self, group_id: String, object_id: String, #[wasm_bindgen(unchecked_param_type = "ObjectBody")] body: JsValue) -> Result<JsValue, JsValue> {
        let body = deserialize(body, "body")?;
        self.inner.edit_object(id(&group_id, "groupId")?, id(&object_id, "objectId")?, body).await.map_err(js_error)?;
        Ok(done())
    }

    /// Deletes an object and the children this device holds.
    #[wasm_bindgen(js_name = deleteObject, unchecked_return_type = "undefined")]
    pub async fn delete_object(&self, group_id: String, object_id: String) -> Result<JsValue, JsValue> {
        self.inner.delete_object(id(&group_id, "groupId")?, id(&object_id, "objectId")?).await.map_err(js_error)?;
        Ok(done())
    }

    /// The group profile's id.
    #[wasm_bindgen(js_name = groupProfileId)]
    pub fn group_profile_id(&self, group_id: String) -> Result<String, JsValue> {
        Ok(self.inner.group_profile_id(id(&group_id, "groupId")?).to_string())
    }

    /// A member profile's id.
    #[wasm_bindgen(js_name = memberProfileId)]
    pub fn member_profile_id(&self, group_id: String, user_id: String) -> Result<String, JsValue> {
        Ok(self.inner.member_profile_id(id(&group_id, "groupId")?, id(&user_id, "userId")?).to_string())
    }

    /// The general expense pool's id.
    #[wasm_bindgen(js_name = generalPoolId)]
    pub fn general_pool_id(&self, group_id: String) -> Result<String, JsValue> {
        Ok(self.inner.general_pool_id(id(&group_id, "groupId")?).to_string())
    }

    // ---- Notes ----

    /// A note's body: its `note_update` children combined.
    #[wasm_bindgen(js_name = readNote, unchecked_return_type = "string")]
    pub async fn read_note(&self, group_id: String, note_id: String) -> Result<JsValue, JsValue> {
        Ok(JsValue::from_str(&self.inner.read_note(id(&group_id, "groupId")?, id(&note_id, "noteId")?).await.map_err(js_error)?))
    }

    /// Ends an editing session with the whole text: one `note_update`
    /// child when something changed. Answers whether one was written.
    #[wasm_bindgen(js_name = saveNote, unchecked_return_type = "boolean")]
    pub async fn save_note(&self, group_id: String, note_id: String, text: String) -> Result<JsValue, JsValue> {
        Ok(JsValue::from_bool(self.inner.save_note(id(&group_id, "groupId")?, id(&note_id, "noteId")?, text).await.map_err(js_error)?))
    }

    // ---- Files ----

    /// Encrypts and uploads a file into an object's slot; the reference
    /// goes into the object's value (the shell edits the object with it).
    #[wasm_bindgen(js_name = uploadFile, unchecked_return_type = "FileRef")]
    pub async fn upload_file(&self, group_id: String, object_id: String, slot: String, bytes: Vec<u8>) -> Result<JsValue, JsValue> {
        serialize(&self.inner.upload_file(id(&group_id, "groupId")?, id(&object_id, "objectId")?, slot, bytes).await.map_err(js_error)?)
    }

    /// Downloads and decrypts a file of an object.
    #[wasm_bindgen(js_name = downloadFile, unchecked_return_type = "Uint8Array")]
    pub async fn download_file(&self, group_id: String, object_id: String, #[wasm_bindgen(unchecked_param_type = "FileRef")] file: JsValue) -> Result<JsValue, JsValue> {
        let file = deserialize(file, "file")?;
        let bytes = self.inner.download_file(id(&group_id, "groupId")?, id(&object_id, "objectId")?, file).await.map_err(js_error)?;
        Ok(Uint8Array::from(bytes.as_slice()).into())
    }
}

// ---- A device key in software ----

/// A device key held in software: the browser's key until the `DeviceKey`
/// seam can wait on WebCrypto. The 32 secret bytes are the shell's to keep
/// (in memory for the session, or in its storage when the browser is
/// remembered); the public half and the Diffie-Hellman step are the seam's
/// methods, so the object is handed to `Core` as its `DeviceKeySeam`.
#[wasm_bindgen(js_name = SoftwareDeviceKey)]
pub struct WasmSoftwareDeviceKey {
    inner: SoftwareDeviceKey,
}

#[wasm_bindgen(js_class = SoftwareDeviceKey)]
impl WasmSoftwareDeviceKey {
    /// A key from its 32 secret bytes.
    #[wasm_bindgen(constructor)]
    pub fn new(secret: &[u8]) -> Result<WasmSoftwareDeviceKey, JsValue> {
        let secret: [u8; 32] = secret.try_into().map_err(|_| invalid("a device key's secret is 32 bytes"))?;
        Ok(WasmSoftwareDeviceKey { inner: SoftwareDeviceKey::from_bytes(secret) })
    }

    /// The public half, 32 bytes.
    #[wasm_bindgen(js_name = publicKeyBytes)]
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.inner.public_key_bytes()
    }

    /// X25519 between the private half and `theirPublic`, 32 bytes.
    #[wasm_bindgen(js_name = dhBytes)]
    pub fn dh_bytes(&self, their_public: &[u8]) -> Result<Vec<u8>, JsValue> {
        if their_public.len() != 32 {
            return Err(invalid("a peer public key is 32 bytes"));
        }
        Ok(self.inner.dh_bytes(their_public.to_vec()))
    }
}

// ---- The server in memory ----

/// The whole server in memory, for the web client's tests: it is a
/// `TransportSeam` (its `send`), so a test opens a `Core` over it and
/// drives a group end to end with no network and no sign-in
/// (`signIn` with a token, then `Core.setSessionToken`).
#[wasm_bindgen(js_name = ModelServer)]
pub struct WasmModelServer {
    inner: Arc<ModelServer>,
}

#[wasm_bindgen(js_class = ModelServer)]
impl WasmModelServer {
    /// A server at the origin, empty, deterministic from the seed (its
    /// first 32 bytes; shorter is padded with zeros).
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: String, seed: &[u8]) -> WasmModelServer {
        WasmModelServer { inner: ModelServer::open(base_url, seed.to_vec()) }
    }

    /// A session token that signs in as the user: what a test hands to
    /// `Core.setSessionToken` instead of a login.
    #[wasm_bindgen(js_name = signIn)]
    pub fn sign_in(&self, token: String, user_id: String) -> Result<(), JsValue> {
        self.inner.sign_in(token, id(&user_id, "userId")?);
        Ok(())
    }

    /// Whether a blob is in the bucket.
    #[wasm_bindgen(js_name = hasBlob)]
    pub fn has_blob(&self, key: String) -> bool {
        self.inner.has_blob(key)
    }

    /// Moves the relay's clock forward.
    pub fn advance(&self, ms: f64) {
        self.inner.advance(ms as i64)
    }

    /// Runs the relay's hourly sweeps once: eviction and request expiry.
    pub fn sweep(&self) {
        self.inner.sweep()
    }

    /// The network is down (every call fails) or back.
    #[wasm_bindgen(js_name = setOutage)]
    pub fn set_outage(&self, outage: bool) {
        self.inner.set_outage(outage)
    }

    /// The transport seam: one request, answered by the models.
    #[wasm_bindgen(unchecked_return_type = "TransportResponse")]
    pub async fn send(&self, #[wasm_bindgen(unchecked_param_type = "TransportRequest")] request: JsValue) -> Result<JsValue, JsValue> {
        let request = request_from_js(&request).map_err(|m| invalid(&format!("request: {m}")))?;
        let response = self.inner.send(request).await.map_err(|e| js_sys::Error::new(&e.to_string()))?;
        Ok(Obj::new().set("status", f64::from(response.status)).set("headers", headers_to_js(&response.headers)).set("body", Uint8Array::from(response.body.as_slice())).done())
    }
}

fn request_from_js(v: &JsValue) -> Result<Request, String> {
    let headers = Array::from(&prop(v, "headers")?).iter().map(|h| Ok(Header { name: string_prop(&h, "name")?, value: string_prop(&h, "value")? })).collect::<Result<Vec<_>, String>>()?;
    let body = prop(v, "body")?;
    let body = body.dyn_ref::<Uint8Array>().ok_or("body: not a Uint8Array")?.to_vec();
    Ok(Request { method: string_prop(v, "method")?, url: string_prop(v, "url")?, headers, body })
}
