package com.deappje.android

import com.deappje.android.shell.EncryptedFileSecrets
import java.io.File
import java.nio.file.Files
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class EncryptedFileSecretsTest : HostCoreTest() {
    private val file = File(Files.createTempDirectory("deappje-secrets").toFile(), "secrets")

    private fun key(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    @Test
    fun setGetDeleteWithNothingInTheClear() {
        val key = key()
        val secrets = EncryptedFileSecrets(file, key)
        assertNull(secrets.getSecret("session_token"))
        secrets.setSecret("session_token", "ory_st_very_secret_token")
        secrets.setSecret("device_id", "7d3b1f2e-0000-4000-8000-000000000001")
        assertEquals("ory_st_very_secret_token", secrets.getSecret("session_token"))
        val onDisk = file.readText()
        assertFalse("the token is not in the file", onDisk.contains("very_secret"))
        assertFalse("the name is not in the file", onDisk.contains("session_token"))
        secrets.setSecret("session_token", "ory_st_another")
        assertEquals("ory_st_another", secrets.getSecret("session_token"))
        secrets.deleteSecret("session_token")
        assertNull(secrets.getSecret("session_token"))
        assertEquals("7d3b1f2e-0000-4000-8000-000000000001", EncryptedFileSecrets(file, key).getSecret("device_id"))
    }

    @Test
    fun anotherKeyOrAnotherNameOpensNothing() {
        val key = key()
        EncryptedFileSecrets(file, key).apply {
            setSecret("a", "first")
            setSecret("b", "second")
        }
        assertNull("another key", EncryptedFileSecrets(file, key()).getSecret("a"))
        // The sealed values swapped between the names: the name is associated data.
        val lines = file.readLines().associate { line -> line.split(' ').let { it[0] to it[1] } }
        val (nameA, nameB) = lines.keys.toList()
        file.writeText("$nameA ${lines[nameB]}\n$nameB ${lines[nameA]}\n")
        val swapped = EncryptedFileSecrets(file, key)
        assertNull(swapped.getSecret("a"))
        assertNull(swapped.getSecret("b"))
    }
}
