package com.deappje.android

import com.deappje.android.shell.SystemClock
import kotlin.math.abs
import org.junit.Assert.assertTrue
import org.junit.Test

class SystemClockTest : HostCoreTest() {
    @Test
    fun theWallClockInMilliseconds() {
        val clock = SystemClock()
        val before = clock.nowMs()
        val system = System.currentTimeMillis()
        val after = clock.nowMs()
        assertTrue("within a second of the JVM's clock", abs(system - before) < 1_000)
        assertTrue("does not run backwards", after >= before)
        assertTrue("milliseconds, not seconds: past 2020", before > 1_577_836_800_000L)
    }
}
