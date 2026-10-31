import DeappjeCore
import Foundation

/// A silent push is a hurry-up, never a message (core/AGENTS.md,
/// "Wake-ups"): its `reason` names a `Core` call the app makes anyway on
/// the foreground and on its timer. The payload is `{"aps": {...}, "wake":
/// {"reason": ..., ids...}}` (nanoservices/device/src/push.rs).
public enum WakeUp: Equatable, Sendable {
    /// `head`, `keys`, `rotate`, `history`: sync.
    case sync
    /// `key_request`: the approver's list, and the approval shown.
    case keyRequest
    /// `history_ready`: hand it to the core, then sync.
    case historyReady(requestId: Uuid, throughSeq: Int64, complete: Bool)

    /// From the push's `userInfo`; nil when it is no wake-up this app knows.
    public init?(userInfo: [AnyHashable: Any]) {
        guard let wake = userInfo["wake"] as? [AnyHashable: Any] else { return nil }
        var payload: [String: String] = [:]
        for (key, value) in wake {
            guard let key = key as? String else { continue }
            payload[key] = value as? String ?? String(describing: value)
        }
        self.init(payload: payload)
    }

    /// From the flattened string map the server sends.
    public init?(payload: [String: String]) {
        switch payload["reason"] {
        case "head", "keys", "rotate", "history":
            self = .sync
        case "key_request":
            self = .keyRequest
        case "history_ready":
            guard let requestId = payload["request_id"],
                let throughSeq = payload["through_seq"].flatMap({ Int64($0) }),
                let complete = payload["complete"].flatMap({ Bool($0) })
            else { return nil }
            self = .historyReady(requestId: requestId, throughSeq: throughSeq, complete: complete)
        default:
            return nil
        }
    }
}
