package com.deappje.android.shell

import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.internal.http.HttpMethod
import uniffi.deappje_core.Header
import uniffi.deappje_core.Request
import uniffi.deappje_core.Response
import uniffi.deappje_core.Transport
import uniffi.deappje_core.TransportException

/**
 * HTTP as the core asks for it (core/src/transport.rs): one request, bytes
 * in and bytes out, over OkHttp. The core sets every header it needs (the
 * content type, the session token), so the body carries no media type of
 * its own; TLS and proxies are OkHttp's. Only the network is an error: a
 * status of any kind is a response, which the core reads.
 */
class OkHttpTransport(private val client: OkHttpClient = OkHttpClient()) : Transport {
    override suspend fun send(request: Request): Response = withContext(Dispatchers.IO) {
        val body = when {
            request.body.isNotEmpty() || HttpMethod.requiresRequestBody(request.method) -> request.body.toRequestBody()
            else -> null
        }
        val built = okhttp3.Request.Builder()
            .url(request.url)
            .method(request.method, body)
            .apply { request.headers.forEach { header(it.name, it.value) } }
            .build()
        try {
            client.newCall(built).execute().use { response ->
                Response(
                    status = response.code.toUShort(),
                    headers = response.headers.map { (name, value) -> Header(name.lowercase(), value) },
                    body = response.body?.bytes() ?: ByteArray(0),
                )
            }
        } catch (e: IOException) {
            throw TransportException.Network(e.toString())
        }
    }
}
