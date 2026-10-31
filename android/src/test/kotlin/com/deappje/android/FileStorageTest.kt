package com.deappje.android

import com.deappje.android.shell.FileStorage
import java.nio.file.Files
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FileStorageTest : HostCoreTest() {
    private val dir = Files.createTempDirectory("deappje-store").toFile()

    @Test
    fun putGetDeleteAndKeysInOrder() {
        val storage = FileStorage(dir)
        assertNull(storage.get("k/g/1"))
        storage.put("k/g/2", byteArrayOf(2))
        storage.put("k/g/1", byteArrayOf(1))
        storage.put("p/device-ä", byteArrayOf(3, 4))
        storage.put("k/g/1", byteArrayOf(9, 9))
        assertArrayEquals(byteArrayOf(9, 9), storage.get("k/g/1"))
        assertArrayEquals(byteArrayOf(3, 4), storage.get("p/device-ä"))
        assertEquals(listOf("k/g/1", "k/g/2"), storage.keys("k/"))
        assertEquals(listOf("k/g/1", "k/g/2", "p/device-ä"), storage.keys(""))
        storage.delete("k/g/1")
        storage.delete("never-there")
        assertNull(storage.get("k/g/1"))
        assertEquals(listOf("k/g/2"), storage.keys("k/"))
    }

    @Test
    fun aReopenedStoreHoldsTheSame() {
        FileStorage(dir).put("o/x", ByteArray(5_000) { it.toByte() })
        val again = FileStorage(dir)
        assertArrayEquals(ByteArray(5_000) { it.toByte() }, again.get("o/x"))
        assertEquals(listOf("o/x"), again.keys(""))
        assertEquals("one file per key, nothing else left behind", 1, dir.list()!!.size)
    }
}
