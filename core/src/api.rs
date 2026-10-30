//! What every call to the server has in common: the answer codes Connect
//! carries and the error the seams (`relay::Relay`, `keys::Devices`)
//! answer with. The engine tells a refusal it must act on from an outage
//! it waits out by the code.

use std::fmt;

/// The server's answer code, as Connect carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// The request is malformed.
    InvalidArgument,
    /// The state does not allow it (an ack backwards, a key id taken).
    FailedPrecondition,
    /// Not a member, not a holder, not the claimer.
    PermissionDenied,
    /// A request that is gone.
    NotFound,
    /// The server failed.
    Internal,
    /// The server could not be reached, or the session is gone: nothing
    /// was decided.
    Unavailable,
}

/// A refusal or a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// The code.
    pub code: Code,
    /// The server's message.
    pub message: String,
}

impl ApiError {
    /// A refusal with the code.
    pub fn new(code: Code, message: impl Into<String>) -> ApiError {
        ApiError { code, message: message.into() }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}
