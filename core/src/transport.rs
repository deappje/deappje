//! HTTP as the shell does it: one request at a time, bytes in and bytes
//! out, nothing of the protocol above it. `fetch` in a browser,
//! `URLSession` on iOS, `OkHttp` on Android; a recorder in the tests. The
//! core adds the headers (the session token, the content type) and reads
//! the status; TLS, proxies and cookies are the shell's.

use std::fmt;

/// One header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct Header {
    /// The name, lowercase.
    pub name: String,
    /// The value.
    pub value: String,
}

impl Header {
    /// A header.
    pub fn new(name: &str, value: impl Into<String>) -> Header {
        Header { name: name.to_owned(), value: value.into() }
    }
}

/// A request as the core hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct Request {
    /// `GET`, `POST`, `PUT`, `DELETE`.
    pub method: String,
    /// The full URL.
    pub url: String,
    /// Headers, in order.
    pub headers: Vec<Header>,
    /// The body; empty for a GET.
    pub body: Vec<u8>,
}

impl Request {
    /// Whether the request carries the header with the value.
    pub fn has_header(&self, name: &str, value: &str) -> bool {
        self.headers.iter().any(|h| h.name.eq_ignore_ascii_case(name) && h.value == value)
    }
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Record))]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// Headers, names lowercased.
    pub headers: Vec<Header>,
    /// The body.
    pub body: Vec<u8>,
}

impl Response {
    /// A header's value, by name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
    }
}

/// Why no response came: the network, not the server.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(uniffi::Error))]
#[cfg_attr(not(target_arch = "wasm32"), uniffi(flat_error))]
pub enum TransportError {
    /// What the platform said.
    Network(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Network(m) => write!(f, "transport: {m}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// The seam. On the phones a UniFFI foreign trait with an async method
/// (uniffi boxes it through async-trait, `Send` off the browser); the
/// core holds it as an `Arc`.
#[cfg_attr(not(target_arch = "wasm32"), uniffi::export(with_foreign))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Transport: crate::shell::MaybeSendSync {
    /// Sends the request and waits for the whole response.
    async fn send(&self, request: Request) -> Result<Response, TransportError>;
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<T: Transport + ?Sized> Transport for std::sync::Arc<T> {
    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        (**self).send(request).await
    }
}
