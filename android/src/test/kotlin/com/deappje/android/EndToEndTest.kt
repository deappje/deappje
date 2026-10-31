package com.deappje.android

import com.deappje.android.shell.SoftwareDeviceKey
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.deappje_core.Core
import uniffi.deappje_core.GroupKind
import uniffi.deappje_core.GroupProfile
import uniffi.deappje_core.ModelServer
import uniffi.deappje_core.ObjectBody
import uniffi.deappje_core.Post
import uniffi.deappje_core.Uuid

/**
 * Two phones of two users against the in-memory server, through the
 * bindings the app uses (docs/01-deappje/03-testing.md, "App shells"): A
 * signs in, registers, makes a group and posts; B signs in, registers, joins
 * through A's link; A approves B's device; B syncs and reads the post, then
 * answers, and A edits and deletes its own.
 */
class EndToEndTest : HostCoreTest() {
    private val origin = "https://app.example"
    private val server = ModelServer.open(origin, ByteArray(32) { 1 })

    private val userA: Uuid = "00000000-0000-0000-0000-000000000001"
    private val userB: Uuid = "00000000-0000-0000-0000-000000000002"

    /** A phone signed in as [user] through a kept session token, its device registered. */
    private suspend fun phone(n: Long, user: Uuid): Core {
        val token = "tok-$n"
        server.signIn(token, user)
        val secrets = MemorySecrets()
        val key = SoftwareDeviceKey.keptOrMinted(secrets, SeededRandom(100 + n))
        val core = Core(server, MemoryStorage(), secrets, key, SeededRandom(n), FixedClock(1_000 + n), origin)
        core.setSessionToken(token)
        val identity = core.restoreSession()
        assertEquals(user, identity!!.id)
        assertEquals(user, core.userId())
        core.registerDevice("android")
        return core
    }

    @Test
    fun twoPhonesShareAGroup() = runBlocking {
        val a = phone(1, userA)
        val b = phone(2, userB)

        // A: a group with a profile and a post, pushed.
        val g = a.createGroup(null)
        a.createObject(g, null, ObjectBody.GroupProfile(GroupProfile("Us", "", ByteArray(0), null, GroupKind.GROUP)))
        val post = a.createObject(g, null, ObjectBody.Post(Post("hello")))
        assertTrue(a.readObject(g, post)!!.pending)
        var report = a.sync()
        assertTrue(report.toString(), report.changed.contains(g))
        assertTrue(report.toString(), report.failed.isEmpty())
        assertFalse(a.readObject(g, post)!!.pending)

        // A's invite link, as the app would show it; B joins through it.
        val link = a.createInvite(g, "member", 0L, 0)
        a.sync()
        val joined = b.join(origin + link)
        assertEquals(g, joined.groupId)
        val groupsOfB = b.listGroups()
        assertEquals(1, groupsOfB.size)
        assertFalse("no key yet", groupsOfB[0].holdsKeys)
        assertTrue("nothing to read without a key", b.listObjects(g).isEmpty())

        // A sees the request, vouched for by the invite, with B's fingerprint.
        val pending = a.pendingKeyRequests()
        assertEquals(pending.toString(), 1, pending.size)
        assertEquals(b.fingerprint(), pending[0].fingerprint)
        assertEquals(userB, pending[0].userId)
        assertEquals("android", pending[0].platform)
        assertEquals(true, pending[0].inviteVerified)
        a.approveKeyRequest(pending[0].requestId)
        assertTrue(a.pendingKeyRequests().isEmpty())

        // B receives the key, catches up and reads the post.
        report = b.sync()
        assertEquals(report.toString(), listOf(g), report.keysReceived)
        assertTrue(b.listGroups()[0].holdsKeys)
        val seen = b.readObject(g, post)
        assertNotNull(seen)
        assertEquals("hello", (seen!!.body as ObjectBody.Post).v1.text)
        assertEquals(userA, seen.author)
        assertEquals("Us", (b.readObject(g, b.groupProfileId(g))!!.body as ObjectBody.GroupProfile).v1.name)

        // B answers; A edits and deletes its own.
        val reply = b.createObject(g, null, ObjectBody.Post(Post("hi!")))
        b.sync()
        a.sync()
        assertEquals("hi!", (a.readObject(g, reply)!!.body as ObjectBody.Post).v1.text)
        a.editObject(g, post, ObjectBody.Post(Post("hello, B")))
        a.sync()
        b.sync()
        assertEquals("hello, B", (b.readObject(g, post)!!.body as ObjectBody.Post).v1.text)
        a.deleteObject(g, post)
        a.sync()
        b.sync()
        assertNull(b.readObject(g, post))
        assertEquals(listOf(reply), b.listObjects(g).filter { it.body is ObjectBody.Post }.map { it.id })
    }
}
