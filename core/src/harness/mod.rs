//! The conformance harness (`docs/01-deappje/03-testing.md`): an in-memory
//! model of `relay`'s contract (`02-relay.md`), with its eviction and
//! wake-ups under a simulated clock, that the sync engine's tests and the
//! shells' end-to-end tests drive several devices against. Public because
//! the shells' tests take it through the bindings.

mod relay_model;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

pub use relay_model::{CLAIM_TAKEOVER_MS, DEFAULT_PAGE, MAX_PAGE, MAX_VALUE_BYTES, RelayModel, Role, WINDOW_MS, Wakeup};

/// Runs a future that never waits (the model answers at once): the tests'
/// executor.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
    }
}
