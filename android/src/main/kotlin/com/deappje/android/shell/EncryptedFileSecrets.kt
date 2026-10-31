package com.deappje.android.shell

import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.GeneralSecurityException
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.deappje_core.Secrets

/**
 * The core's secrets (core/src/shell.rs, `Secrets`: the device id, the
 * session token, the software device key) in one file, every value sealed
 * with AES-256-GCM under a key the file never holds: on a phone a key of the
 * Android Keystore (`keystore/KeystoreKeys.kt`), in a test a key in memory.
 * The name is the associated data, so a sealed value cannot be moved under
 * another name. One line per secret, `base64(name) base64(iv ‖ ciphertext)`;
 * the file is rewritten whole on every change, through a temporary file
 * moved over it. A value that no longer opens (another key, a damaged file)
 * reads as absent.
 */
class EncryptedFileSecrets(private val file: File, private val key: SecretKey) : Secrets {
    private val sealed = LinkedHashMap<String, ByteArray>()

    init {
        if (file.isFile) {
            for (line in file.readLines()) {
                val parts = line.split(' ')
                if (parts.size != 2) continue
                sealed[String(Base64.getDecoder().decode(parts[0]))] = Base64.getDecoder().decode(parts[1])
            }
        }
    }

    @Synchronized
    override fun getSecret(name: String): String? = sealed[name]?.let { open(name, it) }

    @Synchronized
    override fun setSecret(name: String, value: String) {
        sealed[name] = seal(name, value)
        save()
    }

    @Synchronized
    override fun deleteSecret(name: String) {
        if (sealed.remove(name) != null) save()
    }

    private fun seal(name: String, value: String): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        // The provider draws the nonce: the Keystore refuses a caller's.
        cipher.init(Cipher.ENCRYPT_MODE, key)
        cipher.updateAAD(name.toByteArray())
        return cipher.iv + cipher.doFinal(value.toByteArray())
    }

    private fun open(name: String, bytes: ByteArray): String? {
        if (bytes.size < NONCE_SIZE) return null
        return try {
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_BITS, bytes, 0, NONCE_SIZE))
            cipher.updateAAD(name.toByteArray())
            String(cipher.doFinal(bytes, NONCE_SIZE, bytes.size - NONCE_SIZE))
        } catch (e: GeneralSecurityException) {
            null
        }
    }

    private fun save() {
        file.parentFile?.mkdirs()
        val temporary = File(file.path + ".tmp")
        temporary.writeText(
            sealed.entries.joinToString("") { (name, bytes) ->
                Base64.getEncoder().encodeToString(name.toByteArray()) + " " + Base64.getEncoder().encodeToString(bytes) + "\n"
            },
        )
        Files.move(temporary.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
    }

    private companion object {
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val NONCE_SIZE = 12
        const val TAG_BITS = 128
    }
}
