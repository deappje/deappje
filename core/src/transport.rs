//! HTTP as the shell does it: one request at a time, bytes in and bytes
//! out, nothing of the protocol above it. `fetch` in a browser,
//! `URLSession` on iOS, `OkHttp` on Android; a recorder in the tests. The
//! core adds the headers (the session token, the content type) and reads
//! the status; TLS, proxies and cookies are the shell's.

use std::fmt;

/// A request as the core hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `GET`, `POST`, `PUT`, `DELETE`.
    pub method: String,
    /// The full URL.
    pub url: String,
    /// Headers, name and value, in order.
    pub headers: Vec<(String, String)>,
    /// The body; empty for a GET.
    pub body: Vec<u8>,
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// Headers, names lowercased.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl Response {
    /// A header's value, by lowercase name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Why no response came: the network, not the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError(pub String);

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "transport: {}", self.0)
    }
}

impl std::error::Error for TransportError {}

/// The seam.
#[allow(async_fn_in_trait)] // the core's own seam
pub trait Transport {
    /// Sends the request and waits for the whole response.
    async fn send(&self, request: Request) -> Result<Response, TransportError>;
}
