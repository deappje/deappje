package com.deappje.android

import java.util.Random as JavaRandom
import java.util.TreeMap
import uniffi.deappje_core.Clock
import uniffi.deappje_core.Random
import uniffi.deappje_core.Secrets
import uniffi.deappje_core.Storage

/** The seams in memory, for a test's device (core/src/shell.rs has the Rust ones). */
class MemorySecrets : Secrets {
    private val values = HashMap<String, String>()

    @Synchronized override fun getSecret(name: String): String? = values[name]

    @Synchronized override fun setSecret(name: String, value: String) {
        values[name] = value
    }

    @Synchronized override fun deleteSecret(name: String) {
        values.remove(name)
    }
}

class MemoryStorage : Storage {
    private val blobs = TreeMap<String, ByteArray>()

    @Synchronized override fun get(key: String): ByteArray? = blobs[key]?.copyOf()

    @Synchronized override fun put(key: String, value: ByteArray) {
        blobs[key] = value.copyOf()
    }

    @Synchronized override fun delete(key: String) {
        blobs.remove(key)
    }

    @Synchronized override fun keys(prefix: String): List<String> = blobs.keys.filter { it.startsWith(prefix) }

    @Synchronized fun size(): Int = blobs.size
}

class FixedClock(private var now: Long) : Clock {
    @Synchronized override fun nowMs(): Long = now

    @Synchronized fun advance(ms: Long) {
        now += ms
    }
}

/** Deterministic entropy: a seeded generator, 32 bytes a call. */
class SeededRandom(seed: Long) : Random {
    private val random = JavaRandom(seed)

    @Synchronized override fun randomBytes(): ByteArray = ByteArray(32).also(random::nextBytes)
}
