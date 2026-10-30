//! The silent wake-ups this nanoservice sends, and the one place they are
//! dispatched from. A push carries no content a person reads: a reason and
//! the ids a client needs to know what to sync (`docs/01-deappje/04-config.md`:
//! "push payload shapes: constants in `device`"). A duplicate costs one
//! extra wake, which is what makes both adapters `Idempotent`.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;

use basable_core::Ctx;
use basable_externaleffect::{DispatchError, Unfenced};
use interfaces::{DeviceRoutes, DeviceSender};
use messages::{CheckMembershipRequest, PushProvider, Role};
use uuid::Uuid;

use crate::Device;
use crate::effects::{SendApnsArgs, SendFcmArgs};
use crate::model;
use crate::provider::Unregistered;
use crate::repository as repo;

/// The `reason` of every wake-up, the first thing a client reads.
pub mod reason {
    /// A group's head advanced: pull.
    pub const HEAD: &str = "head";
    /// A device asked for the group's keys: show the approval.
    pub const KEY_REQUEST: &str = "key_request";
    /// The keys the device asked for arrived: fetch them.
    pub const KEYS: &str = "keys";
    /// A member left: one holder mints and publishes the next key.
    pub const ROTATE: &str = "rotate";
    /// A history request to answer, if this device holds the range.
    pub const HISTORY: &str = "history";
    /// Rows for the device's own history request landed: page them.
    pub const HISTORY_READY: &str = "history_ready";
}

/// What a wake-up says, before it is flattened to the string map both
/// push services carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wakeup {
    Head { group_id: Uuid, head_seq: i64 },
    KeyRequest { group_id: Uuid, request_id: Uuid },
    Keys { group_id: Uuid, request_id: Uuid },
    Rotate { group_id: Uuid, removed_user: Uuid },
    History { request_id: Uuid, group_id: Uuid, from_seq: i64, to_seq: i64, object_id: Option<Uuid> },
    HistoryReady { request_id: Uuid, through_seq: i64, complete: bool },
}

impl Wakeup {
    pub fn reason(&self) -> &'static str {
        match self {
            Wakeup::Head { .. } => reason::HEAD,
            Wakeup::KeyRequest { .. } => reason::KEY_REQUEST,
            Wakeup::Keys { .. } => reason::KEYS,
            Wakeup::Rotate { .. } => reason::ROTATE,
            Wakeup::History { .. } => reason::HISTORY,
            Wakeup::HistoryReady { .. } => reason::HISTORY_READY,
        }
    }

    /// The payload: `reason` plus the ids, every value a string (FCM's
    /// data messages carry nothing else).
    pub fn data(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("reason".to_owned(), self.reason().to_owned());
        match self {
            Wakeup::Head { group_id, head_seq } => {
                m.insert("group_id".into(), group_id.to_string());
                m.insert("head_seq".into(), head_seq.to_string());
            }
            Wakeup::KeyRequest { group_id, request_id } | Wakeup::Keys { group_id, request_id } => {
                m.insert("group_id".into(), group_id.to_string());
                m.insert("request_id".into(), request_id.to_string());
            }
            Wakeup::Rotate { group_id, .. } => {
                m.insert("group_id".into(), group_id.to_string());
            }
            Wakeup::History { request_id, group_id, from_seq, to_seq, object_id } => {
                m.insert("request_id".into(), request_id.to_string());
                m.insert("group_id".into(), group_id.to_string());
                m.insert("from_seq".into(), from_seq.to_string());
                m.insert("to_seq".into(), to_seq.to_string());
                if let Some(id) = object_id {
                    m.insert("object_id".into(), id.to_string());
                }
            }
            Wakeup::HistoryReady { request_id, through_seq, complete } => {
                m.insert("request_id".into(), request_id.to_string());
                m.insert("through_seq".into(), through_seq.to_string());
                m.insert("complete".into(), complete.to_string());
            }
        }
        m
    }

    /// The effect's identity for one device: the event and the device,
    /// never a retry count.
    pub fn key(&self, device_id: Uuid) -> String {
        match self {
            Wakeup::Head { group_id, head_seq } => format!("head:{device_id}:{group_id}:{head_seq}"),
            Wakeup::KeyRequest { request_id, .. } => format!("key_request:{device_id}:{request_id}"),
            Wakeup::Keys { request_id, .. } => format!("keys:{device_id}:{request_id}"),
            Wakeup::Rotate { group_id, removed_user } => format!("rotate:{device_id}:{group_id}:{removed_user}"),
            Wakeup::History { request_id, .. } => format!("history:{device_id}:{request_id}"),
            Wakeup::HistoryReady { request_id, through_seq, .. } => format!("history_ready:{device_id}:{request_id}:{through_seq}"),
        }
    }
}

/// Whether a dispatch failure (anywhere in its chain) is the provider
/// saying the token is dead.
fn is_unregistered(err: &(dyn Error + 'static)) -> bool {
    let mut current: Option<&(dyn Error + 'static)> = Some(err);
    while let Some(e) = current {
        if e.is::<Unregistered>() {
            return true;
        }
        current = e.source();
    }
    false
}

impl Device {
    /// Sends one wake-up to one device, `Unfenced` (a request path holds
    /// no claim). A token the provider reports dead is forgotten; any
    /// other failure is logged and dropped, the client polls heads on
    /// foreground anyway. A device without a token is simply not woken.
    pub(crate) async fn wake(&self, ctx: &Ctx, device: &model::Device, wakeup: &Wakeup) {
        let (Some(provider), Some(token)) = (device.push_provider.as_deref(), device.push_token.as_deref()) else {
            return;
        };
        let key = wakeup.key(device.id);
        let data = wakeup.data();
        let outcome = match provider.parse::<PushProvider>() {
            Ok(PushProvider::Apns) => self
                .calls
                .send_apns
                .dispatch(ctx, &Unfenced, SendApnsArgs { key, token: token.to_owned(), data })
                .await
                .map(|_| ()),
            Ok(PushProvider::Fcm) => self
                .calls
                .send_fcm
                .dispatch(ctx, &Unfenced, SendFcmArgs { key, token: token.to_owned(), data })
                .await
                .map(|_| ()),
            Err(e) => {
                tracing::warn!(device = %device.id, error = %e, "device carries an unknown push provider");
                return;
            }
        };
        match outcome {
            Ok(()) => {}
            Err(DispatchError::Definitive(e)) if is_unregistered(&*e) => {
                tracing::info!(device = %device.id, "push token unregistered: forgotten");
                if let Err(e) = repo::clear_push_token(&*self.pool, device.id, token).await {
                    tracing::warn!(device = %device.id, error = %e, "could not forget the push token");
                }
            }
            Err(e) => {
                tracing::warn!(device = %device.id, reason = wakeup.reason(), error = %e, "push not delivered");
            }
        }
    }

    /// Wakes every listed device.
    pub(crate) async fn wake_all(&self, ctx: &Ctx, devices: &[model::Device], wakeup: &Wakeup) {
        for d in devices {
            self.wake(ctx, d, wakeup).await;
        }
    }

    /// The group's holder devices whose user is still a member, `except`
    /// left out: who a group-wide wake-up goes to. Membership is `group`'s
    /// to answer (one question per distinct user).
    pub(crate) async fn member_holders<R: DeviceRoutes>(
        &self,
        ctx: &Ctx,
        s: &DeviceSender<'_, R>,
        group_id: Uuid,
        except: &[Uuid],
    ) -> Result<Vec<model::Device>, basable_core::AppError> {
        let holders = repo::holder_devices(&*self.pool, group_id)
            .await
            .map_err(|e| basable_core::AppError::wrap(basable_core::Code::Internal, "device: holders", e))?;
        let mut roles: HashMap<Uuid, Option<Role>> = HashMap::new();
        let mut out = Vec::new();
        for d in holders {
            if except.contains(&d.id) {
                continue;
            }
            let role = match roles.get(&d.user_id) {
                Some(r) => *r,
                None => {
                    let r = s
                        .send_check_membership_request(ctx, CheckMembershipRequest { user_id: d.user_id, group_id })
                        .await?
                        .role;
                    roles.insert(d.user_id, r);
                    r
                }
            };
            if role.is_some() {
                out.push(d);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_name_their_reason_and_ids() {
        let g = Uuid::from_u128(1);
        let r = Uuid::from_u128(2);
        let d = Uuid::from_u128(3);
        let head = Wakeup::Head { group_id: g, head_seq: 7 };
        assert_eq!(head.data().get("reason").map(String::as_str), Some("head"));
        assert_eq!(head.data().get("head_seq").map(String::as_str), Some("7"));
        assert_eq!(head.key(d), format!("head:{d}:{g}:7"));
        let hist = Wakeup::History { request_id: r, group_id: g, from_seq: 1, to_seq: 9, object_id: None };
        assert!(!hist.data().contains_key("object_id"));
        assert_eq!(hist.key(d), format!("history:{d}:{r}"));
        let ready = Wakeup::HistoryReady { request_id: r, through_seq: 5, complete: true };
        assert_eq!(ready.data().get("complete").map(String::as_str), Some("true"));
        assert_ne!(Wakeup::Keys { group_id: g, request_id: r }.key(d), Wakeup::KeyRequest { group_id: g, request_id: r }.key(d));
    }
}
