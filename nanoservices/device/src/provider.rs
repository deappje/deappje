//! The provider seam: the narrow trait `effects.rs` sends through, the two
//! real clients (APNs over HTTP/2 with a provider token, FCM's HTTP v1 API
//! with a service-account access token) and the pair that holds whichever
//! of them is configured. A provider answers with a `BoxError` that
//! carries its own verdict on its own call: `definitive(..)` for a parsed
//! 4xx — the receiver's structured answer — and `TransportError::wrap(..)`
//! for everything whose outcome is unknowable (a transport error, a 5xx, a
//! timeout), which every classifier holds ambiguous. A dead token is a
//! definitive [`Unregistered`], which `push.rs` turns into a forgotten
//! token.

use std::fmt;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use basable_core::{BoxError, Ctx};
use basable_externaleffect::{TransportError, definitive};
use reqwest::StatusCode;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, RSA_PKCS1_SHA256, RsaKeyPair};

use crate::effects::*;

/// Exactly the calls this nanoservice makes — never a wrapper around the
/// whole provider API. The futures are `Send` because a worker dispatches
/// them.
pub trait Provider: Send + Sync + 'static {
    fn send_apns(&self, ctx: Ctx, args: SendApnsArgs) -> impl Future<Output = Result<SendApnsResult, BoxError>> + Send;
    fn send_fcm(&self, ctx: Ctx, args: SendFcmArgs) -> impl Future<Output = Result<SendFcmResult, BoxError>> + Send;
}

/// The provider's word that the token no longer reaches a device (APNs
/// 410 or `BadDeviceToken`, FCM `UNREGISTERED`): definitive, and the one
/// refusal the sender acts on.
#[derive(Debug)]
pub struct Unregistered;

impl fmt::Display for Unregistered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("push token unregistered")
    }
}

impl std::error::Error for Unregistered {}

/// How long a signed token is reused: APNs allows an hour, Google's access
/// tokens last an hour; both are refreshed with ten minutes to spare.
const TOKEN_LIFETIME: Duration = Duration::from_secs(50 * 60);

/// The verdict on a status the caller did not already handle: a 5xx or
/// 429 is unknowable, anything else is the receiver's answer.
fn refused(what: &str, status: StatusCode, body: &str) -> BoxError {
    let message = format!("{what}: {status}: {body}");
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        TransportError::wrap(message)
    } else {
        definitive(message)
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The DER inside a PEM block, whatever its label.
fn pem_der(pem: &str) -> Result<Vec<u8>, BoxError> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .map(str::trim)
        .collect();
    STANDARD.decode(body).map_err(|e| format!("not a PEM key: {e}").into())
}

/// `base64url(header).base64url(claims).base64url(sign(..))`.
fn jwt(header: &serde_json::Value, claims: &serde_json::Value, sign: impl FnOnce(&[u8]) -> Result<Vec<u8>, BoxError>) -> Result<String, BoxError> {
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(header)?),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims)?)
    );
    let signature = sign(signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// A token and when it was minted, behind a lock never held across an
/// await.
#[derive(Default)]
struct TokenCache(Mutex<Option<(String, Instant)>>);

impl TokenCache {
    fn get(&self) -> Option<String> {
        let guard = self.0.lock().unwrap_or_else(|p| p.into_inner());
        guard
            .as_ref()
            .filter(|(_, at)| at.elapsed() < TOKEN_LIFETIME)
            .map(|(t, _)| t.clone())
    }

    fn put(&self, token: String) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some((token, Instant::now()));
    }
}

// --- APNs ---

/// Apple Push Notification service, HTTP/2 with a provider token (a JWT
/// signed ES256 with the account's push key, `APNS_KEY_P8`, under
/// `APNS_KEY_ID` and `APNS_TEAM_ID`; the topic is `APNS_BUNDLE_ID`).
/// `APNS_HOST` picks the sandbox for a development build.
pub struct ApnsClient {
    host: String,
    key: EcdsaKeyPair,
    key_id: String,
    team_id: String,
    bundle_id: String,
    client: reqwest::Client,
    rng: SystemRandom,
    token: TokenCache,
}

impl ApnsClient {
    /// The client from the environment, `None` when the four variables are
    /// absent, an error when they are present and unusable.
    pub fn from_env() -> Result<Option<ApnsClient>, BoxError> {
        let vars = ["APNS_KEY_P8", "APNS_KEY_ID", "APNS_TEAM_ID", "APNS_BUNDLE_ID"].map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()));
        let [Some(p8), Some(key_id), Some(team_id), Some(bundle_id)] = vars else {
            return Ok(None);
        };
        let host = std::env::var("APNS_HOST").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "https://api.push.apple.com".to_owned());
        Ok(Some(ApnsClient::new(host, &p8, key_id, team_id, bundle_id)?))
    }

    pub fn new(host: String, key_p8: &str, key_id: String, team_id: String, bundle_id: String) -> Result<ApnsClient, BoxError> {
        let rng = SystemRandom::new();
        let der = pem_der(key_p8)?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &rng)
            .map_err(|e| format!("APNS_KEY_P8 is not a PKCS#8 P-256 key: {e}"))?;
        Ok(ApnsClient {
            host: host.trim_end_matches('/').to_owned(),
            key,
            key_id,
            team_id,
            bundle_id,
            client: reqwest::Client::builder().http2_prior_knowledge().build()?,
            rng,
            token: TokenCache::default(),
        })
    }

    fn bearer(&self) -> Result<String, BoxError> {
        if let Some(t) = self.token.get() {
            return Ok(t);
        }
        let token = jwt(
            &serde_json::json!({ "alg": "ES256", "kid": self.key_id }),
            &serde_json::json!({ "iss": self.team_id, "iat": unix_now() }),
            |input| {
                self.key
                    .sign(&self.rng, input)
                    .map(|s| s.as_ref().to_vec())
                    .map_err(|e| format!("APNs token signature: {e}").into())
            },
        )?;
        self.token.put(token.clone());
        Ok(token)
    }

    async fn send(&self, args: SendApnsArgs) -> Result<SendApnsResult, BoxError> {
        let bearer = self.bearer()?;
        let body = serde_json::json!({ "aps": { "content-available": 1 }, "wake": args.data });
        let response = self
            .client
            .post(format!("{}/3/device/{}", self.host, args.token))
            .header("authorization", format!("bearer {bearer}"))
            .header("apns-topic", &self.bundle_id)
            .header("apns-push-type", "background")
            .header("apns-priority", "5")
            .json(&body)
            .send()
            .await
            .map_err(TransportError::wrap)?;
        let status = response.status();
        let apns_id = response
            .headers()
            .get("apns-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if status.is_success() {
            return Ok(SendApnsResult { apns_id });
        }
        let body = response.text().await.unwrap_or_default();
        let reason = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_owned))
            .unwrap_or_default();
        if status == StatusCode::GONE || reason == "BadDeviceToken" || reason == "DeviceTokenNotForTopic" {
            return Err(definitive(Unregistered));
        }
        Err(refused("apns", status, &body))
    }
}

// --- FCM ---

/// Firebase Cloud Messaging, HTTP v1, authenticated with an access token
/// minted from a service account (`FCM_SERVICE_ACCOUNT_JSON`: the JSON
/// file Firebase hands out, with the Messaging role).
pub struct FcmClient {
    project_id: String,
    client_email: String,
    token_uri: String,
    key: RsaKeyPair,
    client: reqwest::Client,
    rng: SystemRandom,
    token: TokenCache,
}

#[derive(serde::Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_owned()
}

#[derive(serde::Deserialize)]
struct AccessToken {
    access_token: String,
}

impl FcmClient {
    /// The client from the environment, `None` when the variable is
    /// absent, an error when it is present and unusable.
    pub fn from_env() -> Result<Option<FcmClient>, BoxError> {
        match std::env::var("FCM_SERVICE_ACCOUNT_JSON").ok().filter(|s| !s.is_empty()) {
            Some(json) => Ok(Some(FcmClient::new(&json)?)),
            None => Ok(None),
        }
    }

    pub fn new(service_account_json: &str) -> Result<FcmClient, BoxError> {
        let account: ServiceAccount = serde_json::from_str(service_account_json).map_err(|e| format!("FCM_SERVICE_ACCOUNT_JSON: {e}"))?;
        let der = pem_der(&account.private_key)?;
        let key = RsaKeyPair::from_pkcs8(&der).map_err(|e| format!("the service account's private key is not PKCS#8 RSA: {e}"))?;
        Ok(FcmClient {
            project_id: account.project_id,
            client_email: account.client_email,
            token_uri: account.token_uri,
            key,
            client: reqwest::Client::new(),
            rng: SystemRandom::new(),
            token: TokenCache::default(),
        })
    }

    async fn access_token(&self) -> Result<String, BoxError> {
        if let Some(t) = self.token.get() {
            return Ok(t);
        }
        let now = unix_now();
        let assertion = jwt(
            &serde_json::json!({ "alg": "RS256", "typ": "JWT" }),
            &serde_json::json!({
                "iss": self.client_email,
                "scope": "https://www.googleapis.com/auth/firebase.messaging",
                "aud": self.token_uri,
                "iat": now,
                "exp": now + 3600,
            }),
            |input| {
                let mut signature = vec![0; self.key.public().modulus_len()];
                self.key
                    .sign(&RSA_PKCS1_SHA256, &self.rng, input, &mut signature)
                    .map_err(|e| format!("service account signature: {e}"))?;
                Ok(signature)
            },
        )?;
        let response = self
            .client
            .post(&self.token_uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={assertion}"))
            .send()
            .await
            .map_err(TransportError::wrap)?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(refused("google oauth", status, &body));
        }
        let token: AccessToken = serde_json::from_str(&body).map_err(|e| format!("google oauth: {e}"))?;
        self.token.put(token.access_token.clone());
        Ok(token.access_token)
    }

    async fn send(&self, args: SendFcmArgs) -> Result<SendFcmResult, BoxError> {
        let access_token = self.access_token().await?;
        let body = serde_json::json!({
            "message": {
                "token": args.token,
                "data": args.data,
                "android": { "priority": "high" },
            }
        });
        let response = self
            .client
            .post(format!("https://fcm.googleapis.com/v1/projects/{}/messages:send", self.project_id))
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await
            .map_err(TransportError::wrap)?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.is_success() {
            let name = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_owned))
                .unwrap_or_default();
            return Ok(SendFcmResult { message_name: name });
        }
        if status == StatusCode::NOT_FOUND || body.contains("UNREGISTERED") {
            return Err(definitive(Unregistered));
        }
        Err(refused("fcm", status, &body))
    }
}

/// Whichever of the two services is configured; the other refuses every
/// send with a definitive answer, so nothing is sent and nothing is
/// ambiguous. Local dev configures neither.
pub struct Providers {
    pub apns: Option<ApnsClient>,
    pub fcm: Option<FcmClient>,
}

impl Provider for Providers {
    async fn send_apns(&self, _ctx: Ctx, args: SendApnsArgs) -> Result<SendApnsResult, BoxError> {
        match &self.apns {
            Some(c) => c.send(args).await,
            None => Err(definitive("device.send_apns: APNs not configured (APNS_KEY_P8, APNS_KEY_ID, APNS_TEAM_ID, APNS_BUNDLE_ID)")),
        }
    }

    async fn send_fcm(&self, _ctx: Ctx, args: SendFcmArgs) -> Result<SendFcmResult, BoxError> {
        match &self.fcm {
            Some(c) => c.send(args).await,
            None => Err(definitive("device.send_fcm: FCM not configured (FCM_SERVICE_ACCOUNT_JSON)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_body_is_decoded_whatever_the_label() {
        let der = pem_der("-----BEGIN PRIVATE KEY-----\nAQID\nBAU=\n-----END PRIVATE KEY-----\n").unwrap();
        assert_eq!(der, vec![1, 2, 3, 4, 5]);
        assert!(pem_der("-----BEGIN KEY-----\n!!!\n-----END KEY-----").is_err());
    }

    #[test]
    fn jwt_is_three_base64url_parts() {
        let token = jwt(&serde_json::json!({ "alg": "none" }), &serde_json::json!({ "iss": "me" }), |input| Ok(input.to_vec())).unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(URL_SAFE_NO_PAD.decode(parts[0]).unwrap(), br#"{"alg":"none"}"#);
        assert_eq!(URL_SAFE_NO_PAD.decode(parts[2]).unwrap(), format!("{}.{}", parts[0], parts[1]).into_bytes());
    }

    #[test]
    fn a_token_is_reused_within_its_lifetime() {
        let cache = TokenCache::default();
        assert!(cache.get().is_none());
        cache.put("t".into());
        assert_eq!(cache.get().as_deref(), Some("t"));
    }
}
