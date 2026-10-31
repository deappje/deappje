package com.deappje.android.shell

import java.math.BigInteger

/**
 * X25519 (RFC 7748) in software, for the device key where the Android
 * Keystore cannot hold one (`keystore/DeviceKeys.kt`) and for the tests.
 * The Montgomery ladder of section 5 over [BigInteger]: a handful of
 * operations per key exchange, so speed is no concern, but neither is it
 * constant-time; the Keystore key is preferred wherever it exists.
 */
object X25519 {
    const val KEY_SIZE = 32

    /** The base point, u = 9. */
    val BASE_POINT: ByteArray = ByteArray(KEY_SIZE).also { it[0] = 9 }

    private val P: BigInteger = BigInteger.TWO.pow(255).subtract(BigInteger.valueOf(19))
    private val A24: BigInteger = BigInteger.valueOf(121665)

    /** The public key of a 32-byte secret. */
    fun publicKey(secret: ByteArray): ByteArray = scalarMult(secret, BASE_POINT)

    /** The shared point of a 32-byte secret and a peer's 32-byte public key. */
    fun scalarMult(scalar: ByteArray, u: ByteArray): ByteArray {
        require(scalar.size == KEY_SIZE) { "an X25519 secret is 32 bytes" }
        require(u.size == KEY_SIZE) { "an X25519 public key is 32 bytes" }
        val k = decodeScalar(scalar)
        val x1 = decodeU(u)
        var x2 = BigInteger.ONE
        var z2 = BigInteger.ZERO
        var x3 = x1
        var z3 = BigInteger.ONE
        var swap = 0
        for (t in 254 downTo 0) {
            val kt = k.testBit(t).toInt()
            swap = swap xor kt
            if (swap == 1) {
                x2 = x3.also { x3 = x2 }
                z2 = z3.also { z3 = z2 }
            }
            swap = kt
            val a = x2.add(z2).mod(P)
            val aa = a.multiply(a).mod(P)
            val b = x2.subtract(z2).mod(P)
            val bb = b.multiply(b).mod(P)
            val e = aa.subtract(bb).mod(P)
            val c = x3.add(z3).mod(P)
            val d = x3.subtract(z3).mod(P)
            val da = d.multiply(a).mod(P)
            val cb = c.multiply(b).mod(P)
            val daPlusCb = da.add(cb)
            val daMinusCb = da.subtract(cb)
            x3 = daPlusCb.multiply(daPlusCb).mod(P)
            z3 = x1.multiply(daMinusCb.multiply(daMinusCb)).mod(P)
            x2 = aa.multiply(bb).mod(P)
            z2 = e.multiply(aa.add(A24.multiply(e))).mod(P)
        }
        if (swap == 1) {
            x2 = x3.also { x3 = x2 }
            z2 = z3.also { z3 = z2 }
        }
        return encodeU(x2.multiply(z2.modPow(P.subtract(BigInteger.TWO), P)).mod(P))
    }

    private fun Boolean.toInt() = if (this) 1 else 0

    private fun decodeScalar(bytes: ByteArray): BigInteger {
        val clamped = bytes.copyOf()
        clamped[0] = (clamped[0].toInt() and 248).toByte()
        clamped[31] = ((clamped[31].toInt() and 127) or 64).toByte()
        return BigInteger(1, clamped.reversedArray())
    }

    private fun decodeU(bytes: ByteArray): BigInteger {
        val masked = bytes.copyOf()
        masked[31] = (masked[31].toInt() and 127).toByte()
        return BigInteger(1, masked.reversedArray())
    }

    private fun encodeU(x: BigInteger): ByteArray {
        val big = x.toByteArray().reversedArray()
        return ByteArray(KEY_SIZE).also { big.copyInto(it, 0, 0, minOf(big.size, KEY_SIZE)) }
    }
}
