package com.deappje.android.keystore

import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.annotation.RequiresApi
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.PublicKey
import java.security.spec.NamedParameterSpec
import java.security.spec.X509EncodedKeySpec
import javax.crypto.KeyAgreement
import uniffi.deappje_core.DeviceKey

/**
 * The device key (docs/01-deappje/02-device.md, "Key exchange protocol") in
 * the Android Keystore: an X25519 pair under one alias, its private half
 * never leaving the Keystore, the Diffie-Hellman step of HPKE run inside it.
 * Android 13 (API 33) brought XDH to the Keystore; a KeyMint without
 * Curve25519 refuses the generation, and then [DeviceKeys] falls back to the
 * software key. Keys cross as the raw 32 bytes of RFC 7748, which are the
 * last 32 of the X.509 SubjectPublicKeyInfo the JCA speaks.
 */
@RequiresApi(Build.VERSION_CODES.TIRAMISU)
class KeystoreDeviceKey private constructor(private val privateKey: PrivateKey, publicKey: PublicKey) : DeviceKey {
    private val publicBytes: ByteArray = raw(publicKey)

    override fun publicKeyBytes(): ByteArray = publicBytes.copyOf()

    override fun dhBytes(theirPublic: ByteArray): ByteArray {
        require(theirPublic.size == 32) { "an X25519 public key is 32 bytes" }
        val their = KeyFactory.getInstance(ALGORITHM).generatePublic(X509EncodedKeySpec(SPKI_PREFIX + theirPublic))
        val agreement = KeyAgreement.getInstance(ALGORITHM, KeystoreKeys.PROVIDER)
        agreement.init(privateKey)
        agreement.doPhase(their, true)
        return agreement.generateSecret()
    }

    companion object {
        const val ALIAS = "deappje.device"
        private const val ALGORITHM = "XDH"

        /** DER of `SubjectPublicKeyInfo { id-X25519, BIT STRING (32 bytes) }` before the key. */
        private val SPKI_PREFIX = byteArrayOf(
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x03, 0x21, 0x00,
        )

        /** The key under the alias, if there is one. */
        fun kept(): KeystoreDeviceKey? {
            val store = KeyStore.getInstance(KeystoreKeys.PROVIDER).apply { load(null) }
            val entry = store.getEntry(ALIAS, null) as? KeyStore.PrivateKeyEntry ?: return null
            return KeystoreDeviceKey(entry.privateKey, entry.certificate.publicKey)
        }

        /** A new key under the alias, or null where this Keystore cannot make one. */
        fun mint(): KeystoreDeviceKey? =
            try {
                val generator = KeyPairGenerator.getInstance(ALGORITHM, KeystoreKeys.PROVIDER)
                generator.initialize(
                    KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_AGREE_KEY)
                        .setAlgorithmParameterSpec(NamedParameterSpec("X25519"))
                        .build(),
                )
                val pair = generator.generateKeyPair()
                KeystoreDeviceKey(pair.private, pair.public)
            } catch (e: Exception) {
                // No Curve25519 in this KeyMint, or a provider refusing the spec.
                null
            }

        private fun raw(publicKey: PublicKey): ByteArray {
            val encoded = publicKey.encoded
            check(encoded.size == SPKI_PREFIX.size + 32) { "an X25519 SubjectPublicKeyInfo is 44 bytes" }
            return encoded.copyOfRange(SPKI_PREFIX.size, encoded.size)
        }
    }
}
