package com.deappje.android

import com.deappje.android.shell.SoftwareDeviceKey
import com.deappje.android.shell.X25519
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

/** The software device key against RFC 7748's vectors. */
class DeviceKeyTest : HostCoreTest() {
    private fun hex(s: String): ByteArray = s.chunked(2).map { it.toInt(16).toByte() }.toByteArray()

    @Test
    fun rfc7748Section5Vector1() {
        val scalar = hex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4")
        val u = hex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")
        assertArrayEquals(hex("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"), X25519.scalarMult(scalar, u))
    }

    @Test
    fun rfc7748Section5Vector2() {
        val scalar = hex("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d")
        val u = hex("e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493")
        assertArrayEquals(hex("95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957"), X25519.scalarMult(scalar, u))
    }

    @Test
    fun rfc7748Section6DiffieHellman() {
        val alice = SoftwareDeviceKey(hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a"))
        val bob = SoftwareDeviceKey(hex("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb"))
        assertArrayEquals(hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"), alice.publicKeyBytes())
        assertArrayEquals(hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"), bob.publicKeyBytes())
        val shared = hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")
        assertArrayEquals(shared, alice.dhBytes(bob.publicKeyBytes()))
        assertArrayEquals(shared, bob.dhBytes(alice.publicKeyBytes()))
    }

    @Test
    fun theKeyKeptInTheSecretsIsTheSameKeyNextTime() {
        val secrets = MemorySecrets()
        val first = SoftwareDeviceKey.keptOrMinted(secrets, SeededRandom(7))
        val again = SoftwareDeviceKey.keptOrMinted(secrets, SeededRandom(8))
        assertArrayEquals(first.publicKeyBytes(), again.publicKeyBytes())
        assertEquals(32, first.publicKeyBytes().size)
        val other = SoftwareDeviceKey.keptOrMinted(MemorySecrets(), SeededRandom(8))
        assertArrayEquals(first.dhBytes(other.publicKeyBytes()), other.dhBytes(first.publicKeyBytes()))
    }
}
