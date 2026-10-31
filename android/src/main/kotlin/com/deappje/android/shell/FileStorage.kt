package com.deappje.android.shell

import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.Base64
import uniffi.deappje_core.Storage

/**
 * The core's local store (core/src/shell.rs, `Storage`) as files under one
 * directory, one file per key. A key holds slashes (`k/<group>/<n>`), so the
 * file is named by the key's UTF-8 in URL-safe base64 without padding, which
 * decodes back to the key for [keys]. A write lands whole: the bytes go to a
 * temporary file that is moved over the old one. The directory is the app's
 * own and excluded from backup; the device's private key is never in it.
 */
class FileStorage(private val dir: File) : Storage {
    init {
        check(dir.isDirectory || dir.mkdirs()) { "cannot create ${dir.path}" }
    }

    override fun get(key: String): ByteArray? = file(key).takeIf { it.isFile }?.readBytes()

    override fun put(key: String, value: ByteArray) {
        val target = file(key)
        val temporary = File(dir, target.name + TEMPORARY)
        temporary.writeBytes(value)
        Files.move(temporary.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
    }

    override fun delete(key: String) {
        file(key).delete()
    }

    override fun keys(prefix: String): List<String> =
        (dir.list() ?: emptyArray())
            .filter { !it.endsWith(TEMPORARY) }
            .map(::decode)
            .filter { it.startsWith(prefix) }
            .sorted()

    private fun file(key: String) = File(dir, encode(key))

    private companion object {
        const val TEMPORARY = ".tmp"

        fun encode(key: String): String = Base64.getUrlEncoder().withoutPadding().encodeToString(key.toByteArray())

        fun decode(name: String): String = String(Base64.getUrlDecoder().decode(name))
    }
}
