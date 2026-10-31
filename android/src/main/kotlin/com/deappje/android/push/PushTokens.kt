package com.deappje.android.push

/**
 * Where a push token comes from. A push is a hurry-up, never a message
 * (core/AGENTS.md, "Wake-ups"): the token goes to the core through
 * `setPushToken("fcm", token)` and a wake-up makes the shell call `sync` or
 * `pendingKeyRequests`, which it does on every foreground anyway. Firebase is
 * not in this build (there is no google-services.json yet, android/README.md);
 * a Firebase-backed source implements this and replaces [NoPush] in
 * `AppGraph`.
 */
interface PushTokenSource {
    /** Hands over the current token, now or when it is first known, and again when it changes. */
    fun tokens(onToken: (provider: String, token: String) -> Unit)
}

/** No push: nothing is ever handed over. */
object NoPush : PushTokenSource {
    override fun tokens(onToken: (provider: String, token: String) -> Unit) {}
}
