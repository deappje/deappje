package com.deappje.android

import com.deappje.android.shell.SecureRandomSource
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class SecureRandomSourceTest : HostCoreTest() {
    @Test
    fun thirtyTwoFreshBytesACall() {
        val random = SecureRandomSource()
        val first = random.randomBytes()
        val second = random.randomBytes()
        assertEquals(32, first.size)
        assertEquals(32, second.size)
        assertFalse("two draws are not the same", first.contentEquals(second))
        assertFalse("a draw is not all zeros", first.all { it == 0.toByte() })
    }
}
