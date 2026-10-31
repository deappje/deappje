package com.deappje.android.shell

import java.util.Base64
import uniffi.deappje_core.DeviceKey
import uniffi.deappje_core.Random
import uniffi.deappje_core.Secrets

/**
 * The device key (core/src/shell.rs, `DeviceKey`) in software: an X25519
 * secret whose bytes live in the shell's [Secrets] store, sealed under the
 * Keystore key there, and whose Diffie-Hellman step runs here. The phone
 * uses it below API 33 or where the Keystore has no Curve25519
 * (`keystore/DeviceKeys.kt`); the tests use it everywhere. Minted once from
 * the core's [Random] and kept for good: a device's public key never changes.
 */
class SoftwareDeviceKey(private val secret: ByteArray) : DeviceKey {
    init {
        require(secret.size == X25519.KEY_SIZE) { "an X25519 secret is 32 bytes" }
    }

    private val public = X25519.publicKey(secret)

    override fun publicKeyBytes(): ByteArray = public.copyOf()

    override fun dhBytes(theirPublic: ByteArray): ByteArray = X25519.scalarMult(secret, theirPublic)

    companion object {
        /** The secret the key's bytes live under, base64. */
        const val SECRET_NAME = "device_key"

        /** The key kept in [secrets], if there is one. */
        fun kept(secrets: Secrets): SoftwareDeviceKey? =
            secrets.getSecret(SECRET_NAME)
                ?.let { Base64.getDecoder().decode(it) }
                ?.takeIf { it.size == X25519.KEY_SIZE }
                ?.let(::SoftwareDeviceKey)

        /** The kept key, or a fresh one from [random] kept from now on. */
        fun keptOrMinted(secrets: Secrets, random: Random): SoftwareDeviceKey {
            kept(secrets)?.let { return it }
            val fresh = random.randomBytes().copyOf(X25519.KEY_SIZE)
            secrets.setSecret(SECRET_NAME, Base64.getEncoder().encodeToString(fresh))
            return SoftwareDeviceKey(fresh)
        }
    }
}
