package com.deappje.android.shell

import uniffi.deappje_core.Clock

/** The wall clock (core/src/shell.rs, `Clock`): milliseconds since the epoch, UTC. */
class SystemClock : Clock {
    override fun nowMs(): Long = System.currentTimeMillis()
}
