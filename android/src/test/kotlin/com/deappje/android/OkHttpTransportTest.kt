package com.deappje.android

import com.deappje.android.shell.OkHttpTransport
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import uniffi.deappje_core.Header
import uniffi.deappje_core.Request
import uniffi.deappje_core.TransportException

class OkHttpTransportTest : HostCoreTest() {
    @Test
    fun aPostGoesOutAsGivenAndTheAnswerComesBackWhole() = runBlocking {
        val server = MockWebServer()
        server.enqueue(MockResponse().setResponseCode(200).addHeader("Content-Type", "application/proto").setBody("ok"))
        server.start()
        try {
            val url = server.url("/api/relay.v1.RelayService/Pull").toString()
            val response = OkHttpTransport().send(
                Request(
                    method = "POST",
                    url = url,
                    headers = listOf(Header("content-type", "application/proto"), Header("x-session-token", "tok"), Header("connect-protocol-version", "1")),
                    body = byteArrayOf(8, 1),
                ),
            )
            assertEquals(200.toUShort(), response.status)
            assertEquals("application/proto", response.headers.first { it.name == "content-type" }.value)
            assertTrue("header names come back lowercased", response.headers.all { it.name == it.name.lowercase() })
            assertArrayEquals("ok".toByteArray(), response.body)
            val recorded = server.takeRequest()
            assertEquals("POST", recorded.method)
            assertEquals("/api/relay.v1.RelayService/Pull", recorded.path)
            assertEquals("tok", recorded.getHeader("x-session-token"))
            assertEquals("1", recorded.getHeader("connect-protocol-version"))
            assertEquals("application/proto", recorded.getHeader("content-type"))
            assertArrayEquals(byteArrayOf(8, 1), recorded.body.readByteArray())
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun aStatusIsAnAnswerAndOnlyTheNetworkIsAnError() = runBlocking {
        val server = MockWebServer()
        server.enqueue(MockResponse().setResponseCode(404))
        server.start()
        val base = server.url("/").toString()
        try {
            val response = OkHttpTransport().send(Request("GET", base + "missing", emptyList(), ByteArray(0)))
            assertEquals(404.toUShort(), response.status)
            assertEquals("GET", server.takeRequest().method)
        } finally {
            server.shutdown()
        }
        try {
            OkHttpTransport().send(Request("POST", base + "api/x", emptyList(), ByteArray(0)))
            fail("a closed server is a network error")
        } catch (e: TransportException.Network) {
            assertTrue(e.message!!.isNotEmpty())
        }
    }
}
