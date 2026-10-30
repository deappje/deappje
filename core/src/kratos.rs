//! Signing in from a native app: Kratos's API flows
//! (`https://www.ory.sh/docs/kratos/self-service/flows/user-login`,
//! "native" clients), served under the deployment's `/.ory/`. The app
//! starts a login flow asking for a session token exchange code, submits
//! the Basable OpenID method, and is told the browser URL; the shell opens
//! it in the system browser, which ends on the app's return URL with a
//! code; the core exchanges the flow's code and that one for the session
//! token, which it keeps in the shell's secret store and sends with every
//! call. The web app signs in through Kratos's browser flows and its
//! cookie instead (`frontend/react/src/Auth.tsx`).

use std::fmt;

use crate::transport::{Header, Request, Transport, TransportError};

/// The OpenID provider the login offers: Basable (`k8s/kratos/configmap.yaml`).
pub const PROVIDER: &str = "basable";

/// A login started: what the browser must open, and what the exchange
/// needs afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct LoginStart {
    /// The flow's id.
    pub flow_id: String,
    /// The URL to open in the system browser.
    pub browser_url: String,
    /// The flow's half of the exchange, kept until the browser returns.
    pub exchange_code: String,
}

/// The signed-in identity, as `whoami` has it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct Identity {
    /// Kratos's identity id.
    pub id: String,
    /// The email trait, when the schema has one.
    pub email: Option<String>,
}

/// Why a step of the login did not go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KratosError {
    /// The network.
    Transport(TransportError),
    /// Kratos answered with a status and a body the step cannot use.
    Unexpected {
        /// The status.
        status: u16,
        /// The body, for the log.
        body: String,
    },
    /// The session is gone (401).
    Unauthenticated,
}

impl fmt::Display for KratosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KratosError::Transport(e) => write!(f, "{e}"),
            KratosError::Unexpected { status, body } => write!(f, "kratos: HTTP {status}: {body}"),
            KratosError::Unauthenticated => f.write_str("kratos: not signed in"),
        }
    }
}

impl std::error::Error for KratosError {}

impl From<TransportError> for KratosError {
    fn from(e: TransportError) -> Self {
        KratosError::Transport(e)
    }
}

#[derive(serde::Deserialize)]
struct Flow {
    id: String,
    #[serde(default)]
    session_token_exchange_code: String,
}

#[derive(serde::Deserialize)]
struct BrowserLocationChange {
    #[serde(default)]
    redirect_browser_to: String,
}

#[derive(serde::Deserialize)]
struct TokenExchange {
    session_token: String,
}

#[derive(serde::Deserialize)]
struct Session {
    identity: SessionIdentity,
}

#[derive(serde::Deserialize)]
struct SessionIdentity {
    id: String,
    #[serde(default)]
    traits: serde_json::Value,
}

/// Kratos's public API behind a transport.
pub struct Kratos<'a, T: Transport> {
    transport: &'a T,
    /// `https://<host>/.ory`, no trailing slash.
    base: String,
}

fn unexpected(status: u16, body: &[u8]) -> KratosError {
    KratosError::Unexpected { status, body: String::from_utf8_lossy(body).chars().take(300).collect() }
}

impl<'a, T: Transport> Kratos<'a, T> {
    /// Over the deployment's origin.
    pub fn new(transport: &'a T, base_url: &str) -> Kratos<'a, T> {
        Kratos { transport, base: format!("{}/.ory", base_url.trim_end_matches('/')) }
    }

    async fn send(&self, method: &str, url: String, headers: Vec<Header>, body: Vec<u8>) -> Result<(u16, Vec<u8>), KratosError> {
        let r = self.transport.send(Request { method: method.to_owned(), url, headers, body }).await?;
        Ok((r.status, r.body))
    }

    /// Starts a native login that ends on `return_to` (the app's deep
    /// link), with the Basable provider: the browser URL to open.
    pub async fn start_login(&self, return_to: &str) -> Result<LoginStart, KratosError> {
        let url = format!("{}/self-service/login/api?return_session_token_exchange_code=true&return_to={}", self.base, urlencode(return_to));
        let (status, body) = self.send("GET", url, vec![Header::new("accept", "application/json")], Vec::new()).await?;
        if status != 200 {
            return Err(unexpected(status, &body));
        }
        let flow: Flow = serde_json::from_slice(&body).map_err(|_| unexpected(status, &body))?;
        let submit = format!("{}/self-service/login?flow={}", self.base, urlencode(&flow.id));
        let form = serde_json::json!({ "method": "oidc", "provider": PROVIDER }).to_string().into_bytes();
        let headers = vec![Header::new("content-type", "application/json"), Header::new("accept", "application/json")];
        let (status, body) = self.send("POST", submit, headers, form).await?;
        // A native OpenID login answers 422 with where the browser must go.
        if status != 422 {
            return Err(unexpected(status, &body));
        }
        let change: BrowserLocationChange = serde_json::from_slice(&body).map_err(|_| unexpected(status, &body))?;
        if change.redirect_browser_to.is_empty() {
            return Err(unexpected(status, &body));
        }
        Ok(LoginStart { flow_id: flow.id, browser_url: change.redirect_browser_to, exchange_code: flow.session_token_exchange_code })
    }

    /// The browser came back with `return_code` (the `code` query
    /// parameter of the return URL): the session token.
    pub async fn finish_login(&self, start: &LoginStart, return_code: &str) -> Result<String, KratosError> {
        let url = format!("{}/sessions/token-exchange?init_code={}&return_to_code={}", self.base, urlencode(&start.exchange_code), urlencode(return_code));
        let (status, body) = self.send("GET", url, vec![Header::new("accept", "application/json")], Vec::new()).await?;
        if status != 200 {
            return Err(unexpected(status, &body));
        }
        let t: TokenExchange = serde_json::from_slice(&body).map_err(|_| unexpected(status, &body))?;
        Ok(t.session_token)
    }

    /// Who the token signs in as.
    pub async fn whoami(&self, session_token: &str) -> Result<Identity, KratosError> {
        let url = format!("{}/sessions/whoami", self.base);
        let headers = vec![Header::new("accept", "application/json"), Header::new(crate::connect::SESSION_TOKEN_HEADER, session_token)];
        let (status, body) = self.send("GET", url, headers, Vec::new()).await?;
        match status {
            200 => {
                let s: Session = serde_json::from_slice(&body).map_err(|_| unexpected(status, &body))?;
                let email = s.identity.traits.get("email").and_then(|e| e.as_str()).map(str::to_owned);
                Ok(Identity { id: s.identity.id, email })
            }
            401 => Err(KratosError::Unauthenticated),
            _ => Err(unexpected(status, &body)),
        }
    }

    /// Ends the session.
    pub async fn logout(&self, session_token: &str) -> Result<(), KratosError> {
        let url = format!("{}/self-service/logout/api", self.base);
        let body = serde_json::json!({ "session_token": session_token }).to_string().into_bytes();
        let headers = vec![Header::new("content-type", "application/json"), Header::new("accept", "application/json")];
        let (status, body) = self.send("DELETE", url, headers, body).await?;
        match status {
            204 | 200 => Ok(()),
            401 => Err(KratosError::Unauthenticated),
            _ => Err(unexpected(status, &body)),
        }
    }
}

/// Percent-encodes a query value.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::tests::FakeTransport;
    use crate::harness::block_on;

    #[test]
    fn the_native_login_goes_through_the_browser_and_the_exchange() {
        let t = FakeTransport::default();
        t.answer(200, "application/json", br#"{"id":"flow-1","session_token_exchange_code":"init-xyz","ui":{}}"#.to_vec());
        t.answer(422, "application/json", br#"{"error":{"id":"browser_location_change_required"},"redirect_browser_to":"https://app.example/.ory/self-service/methods/oidc/auth/flow-1"}"#.to_vec());
        t.answer(200, "application/json", br#"{"session_token":"ory_st_secret","session":{"id":"s"}}"#.to_vec());
        t.answer(200, "application/json", br#"{"id":"s","identity":{"id":"ident-1","traits":{"email":"a@example.com"}}}"#.to_vec());
        t.answer(401, "application/json", br#"{"error":{"code":401}}"#.to_vec());
        let k = Kratos::new(&t, "https://app.example/");
        let start = block_on(k.start_login("deappje://login")).unwrap();
        assert_eq!(start, LoginStart { flow_id: "flow-1".into(), browser_url: "https://app.example/.ory/self-service/methods/oidc/auth/flow-1".into(), exchange_code: "init-xyz".into() });
        assert_eq!(block_on(k.finish_login(&start, "ret-123")).unwrap(), "ory_st_secret");
        assert_eq!(block_on(k.whoami("ory_st_secret")).unwrap(), Identity { id: "ident-1".into(), email: Some("a@example.com".into()) });
        assert_eq!(block_on(k.whoami("ory_st_secret")).unwrap_err(), KratosError::Unauthenticated);
        let sent = t.sent();
        assert_eq!(sent[0].url, "https://app.example/.ory/self-service/login/api?return_session_token_exchange_code=true&return_to=deappje%3A%2F%2Flogin");
        assert_eq!(sent[1].url, "https://app.example/.ory/self-service/login?flow=flow-1");
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&sent[1].body).unwrap()["provider"], "basable");
        assert_eq!(sent[2].url, "https://app.example/.ory/sessions/token-exchange?init_code=init-xyz&return_to_code=ret-123");
        assert!(sent[3].has_header("x-session-token", "ory_st_secret"));
    }

    #[test]
    fn an_unexpected_answer_is_reported_with_its_status() {
        let t = FakeTransport::default();
        t.answer(500, "text/plain", b"boom".to_vec());
        let k = Kratos::new(&t, "https://app.example");
        assert_eq!(block_on(k.start_login("deappje://login")).unwrap_err(), KratosError::Unexpected { status: 500, body: "boom".into() });
    }
}
