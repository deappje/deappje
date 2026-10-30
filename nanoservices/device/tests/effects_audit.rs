//! The audit index: one `audit!` per adapter in effects.rs, against the
//! simulator. Green on day one; the first thing a real provider must keep
//! green. Ack-loss probes are mandatory for irreversible adapters.

use std::collections::BTreeMap;
use std::sync::Arc;

use basable_effecttest::{Harness, audit};
use device::effects::*;
use device::simulator::{Fault, Simulator};

fn data() -> BTreeMap<String, String> {
    BTreeMap::from([("reason".to_owned(), "head".to_owned())])
}

audit!(send_apns, || async {
    let sim = Arc::new(Simulator::new());
    let calls = Calls::new(sim.clone());
    let landed = sim.clone();
    let definitive = sim.clone();
    let ambiguity = sim.clone();
    Harness::new(
        calls.send_apns.clone(),
        || SendApnsArgs {
            key: "send_apns-1".into(),
            token: "apns-token".into(),
            data: data(),
        },
        move || {
            let sim = landed.clone();
            async move { sim.landed_count("send_apns") }
        },
        move || {
            let sim = definitive.clone();
            async move { sim.inject("send_apns", Fault::Refused, 1) }
        },
    )
    .inject_ambiguity(move || {
        let sim = ambiguity.clone();
        async move { sim.inject("send_apns", Fault::AckLoss, 1) }
    })
    // A convergent effect: a late duplicate is one extra silent wake.
    .advance_intent(|a| a)
});

audit!(send_fcm, || async {
    let sim = Arc::new(Simulator::new());
    let calls = Calls::new(sim.clone());
    let landed = sim.clone();
    let definitive = sim.clone();
    let ambiguity = sim.clone();
    Harness::new(
        calls.send_fcm.clone(),
        || SendFcmArgs {
            key: "send_fcm-1".into(),
            token: "fcm-token".into(),
            data: data(),
        },
        move || {
            let sim = landed.clone();
            async move { sim.landed_count("send_fcm") }
        },
        move || {
            let sim = definitive.clone();
            async move { sim.inject("send_fcm", Fault::Refused, 1) }
        },
    )
    .inject_ambiguity(move || {
        let sim = ambiguity.clone();
        async move { sim.inject("send_fcm", Fault::AckLoss, 1) }
    })
    .advance_intent(|a| a)
});
