package com.deappje.android.shell

import java.security.SecureRandom
import uniffi.deappje_core.Random

/**
 * The core's entropy (core/src/shell.rs, `Random`): the platform's
 * [SecureRandom], 32 bytes a call. The core seeds its own generator from it
 * and asks again only when it reseeds.
 */
class SecureRandomSource(private val random: SecureRandom = SecureRandom()) : Random {
    override fun randomBytes(): ByteArray = ByteArray(32).also(random::nextBytes)
}
