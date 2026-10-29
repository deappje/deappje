//! The audit index: one `audit!` per adapter in effects.rs, against the
//! simulator. Green on day one; the first thing a real provider must keep
//! green. Ack-loss probes are mandatory for irreversible adapters.

use std::sync::Arc;

use basable_effecttest::{Harness, audit};
use group::effects::*;
use group::simulator::{Fault, Simulator};

audit!(delete_identity, || async {
    let sim = Arc::new(Simulator::new());
    let calls = Calls::new(sim.clone());
    let landed = sim.clone();
    let definitive = sim.clone();
    let ambiguity = sim.clone();
    Harness::new(
        calls.delete_identity.clone(),
        || DeleteIdentityArgs {
            key: "delete_identity-1".into(),
        },
        move || {
            let sim = landed.clone();
            async move { sim.landed_count("delete_identity") }
        },
        move || {
            let sim = definitive.clone();
            async move { sim.inject("delete_identity", Fault::Refused, 1) }
        },
    )
    .inject_ambiguity(move || {
        let sim = ambiguity.clone();
        async move { sim.inject("delete_identity", Fault::AckLoss, 1) }
    })
    // A convergent effect with no intent scope in its args: the only
    // possible late call is a duplicate.
    .advance_intent(|a| a)
});
