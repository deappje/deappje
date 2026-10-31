package com.deappje.android.ui

import android.net.Uri
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import com.deappje.android.AppGraph
import com.deappje.android.Environment
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import uniffi.deappje_core.Core
import uniffi.deappje_core.CoreException
import uniffi.deappje_core.GroupKind
import uniffi.deappje_core.GroupProfile
import uniffi.deappje_core.GroupView
import uniffi.deappje_core.LoginStart
import uniffi.deappje_core.ObjectBody
import uniffi.deappje_core.ObjectView
import uniffi.deappje_core.PendingRequestView
import uniffi.deappje_core.Post
import uniffi.deappje_core.Uuid

/** A group as the list shows it: the core's view and its profile's name, when this device can read it. */
data class GroupRow(val view: GroupView, val name: String?)

/** A post as the timeline shows it. */
data class PostRow(val view: ObjectView, val text: String, val own: Boolean)

/** Where the app is. */
sealed interface Screen {
    data object Groups : Screen

    data object Approvals : Screen

    data class Group(val id: Uuid) : Screen
}

data class UiState(
    val deviceKeyBackend: String,
    val environment: String = Environment.NAME,
    val restoring: Boolean = true,
    val signedIn: Boolean = false,
    val email: String? = null,
    val userId: Uuid? = null,
    val fingerprint: String = "",
    val screen: Screen = Screen.Groups,
    val groups: List<GroupRow> = emptyList(),
    val pending: List<PendingRequestView> = emptyList(),
    val posts: List<PostRow> = emptyList(),
    val inviteLink: String? = null,
    val busy: Boolean = false,
    val notice: String? = null,
)

/**
 * The screens' state and every call into the core (docs/01-deappje/
 * 01-architecture.md, "The shells"): the shell renders what the core holds
 * and does what only the platform can, the browser for the sign-in here.
 * The core serialises its own calls; this model only sequences the ones a
 * tap asks for.
 */
class AppViewModel(private val graph: AppGraph) : ViewModel() {
    private val core: Core = graph.core
    private val _state = MutableStateFlow(UiState(deviceKeyBackend = graph.deviceKeyBackend))
    val state: StateFlow<UiState> = _state

    /** The sign-in the browser is out on, kept until it returns with the code. */
    private var loginStart: LoginStart? = null

    init {
        viewModelScope.launch {
            _state.update { it.copy(fingerprint = core.fingerprint()) }
            attempt {
                val identity = core.restoreSession()
                if (identity != null) signedIn(identity.email) else _state.update { it.copy(restoring = false) }
            }
            _state.update { it.copy(restoring = false) }
        }
        graph.push.tokens { provider, token ->
            viewModelScope.launch { attempt { core.setPushToken(provider, token) } }
        }
    }

    // ---- Signing in ----

    /** Starts the native sign-in; [open] takes the browser URL to a Custom Tab. */
    fun signIn(open: (Uri) -> Unit) = act {
        val start = core.startLogin(RETURN_TO)
        loginStart = start
        open(Uri.parse(start.browserUrl))
    }

    /** The browser came back to deappje://auth with the code. */
    fun finishLogin(code: String) = act {
        val start = loginStart
        if (start == null) {
            notice("The sign-in did not start in this app; try again.")
            return@act
        }
        val identity = core.finishLogin(start, code)
        loginStart = null
        signedIn(identity.email)
    }

    fun logout() = act {
        core.logout()
        _state.update { UiState(deviceKeyBackend = it.deviceKeyBackend, restoring = false, fingerprint = it.fingerprint) }
    }

    private suspend fun signedIn(email: String?) {
        _state.update { it.copy(signedIn = true, email = email, userId = core.userId(), restoring = false) }
        core.registerDevice(PLATFORM)
        refreshNow()
    }

    // ---- Keeping up ----

    /** What the foreground and the timer ask for: one sync, then every list re-read. */
    fun refresh() {
        if (!_state.value.signedIn) return
        viewModelScope.launch { attempt { refreshNow() } }
    }

    private suspend fun refreshNow() {
        val report = core.sync()
        val groups = core.listGroups().map { g ->
            val profile = if (g.holdsKeys) core.readObject(g.id, core.groupProfileId(g.id))?.body as? ObjectBody.GroupProfile else null
            GroupRow(g, profile?.v1?.name?.takeIf { it.isNotBlank() })
        }
        val pending = core.pendingKeyRequests()
        val posts = (_state.value.screen as? Screen.Group)?.let { posts(it.id) } ?: emptyList()
        _state.update { it.copy(groups = groups, pending = pending, posts = posts) }
        if (report.pinAlerts.isNotEmpty()) {
            notice("A known device offers another key: " + report.pinAlerts.joinToString { "${it.deviceId} (${it.offered})" })
        } else if (report.failed.isNotEmpty()) {
            notice("Sync: " + report.failed.joinToString())
        }
    }

    private suspend fun posts(groupId: Uuid): List<PostRow> {
        val me = _state.value.userId
        return core.listObjects(groupId)
            .mapNotNull { o -> (o.body as? ObjectBody.Post)?.let { PostRow(o, it.v1.text, o.author == me) } }
            .sortedByDescending { it.view.authoredAtMs }
    }

    // ---- Groups ----

    fun createGroup(name: String) = act {
        val id = core.createGroup(null)
        core.createObject(id, null, ObjectBody.GroupProfile(GroupProfile(name.trim(), "", ByteArray(0), null, GroupKind.GROUP)))
        refreshNow()
    }

    /** Joins through a pasted link; a member of the group approves this device next. */
    fun join(link: String) = act {
        core.join(link.trim())
        notice("Joined; waiting for a member to approve this device. Your code: ${_state.value.fingerprint}")
        refreshNow()
    }

    fun approve(requestId: Uuid) = act {
        core.approveKeyRequest(requestId)
        refreshNow()
    }

    fun show(screen: Screen) {
        _state.update { it.copy(screen = screen, posts = emptyList(), inviteLink = null) }
        if (screen is Screen.Group) viewModelScope.launch { attempt { _state.update { it.copy(posts = posts(screen.id)) } } }
    }

    fun invite() = act {
        val group = _state.value.screen as? Screen.Group ?: return@act
        val link = core.createInvite(group.id, "member", 0L, 0)
        core.sync()
        _state.update { it.copy(inviteLink = Environment.BASE_URL + link) }
    }

    fun dismissInvite() = _state.update { it.copy(inviteLink = null) }

    // ---- Posts ----

    fun post(text: String) = inGroup { g -> core.createObject(g, null, ObjectBody.Post(Post(text.trim()))) }

    fun editPost(id: Uuid, text: String) = inGroup { g -> core.editObject(g, id, ObjectBody.Post(Post(text.trim()))) }

    fun deletePost(id: Uuid) = inGroup { g -> core.deleteObject(g, id) }

    private fun inGroup(write: suspend (Uuid) -> Unit) = act {
        val group = _state.value.screen as? Screen.Group ?: return@act
        write(group.id)
        refreshNow()
    }

    // ---- Plumbing ----

    fun dismissNotice() = _state.update { it.copy(notice = null) }

    private fun notice(text: String) = _state.update { it.copy(notice = text) }

    private fun act(block: suspend () -> Unit) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true) }
            try {
                attempt(block)
            } finally {
                _state.update { it.copy(busy = false) }
            }
        }
    }

    private suspend fun attempt(block: suspend () -> Unit) {
        try {
            block()
        } catch (e: CoreException) {
            notice(describe(e))
        }
    }

    private fun describe(e: CoreException): String = when (e) {
        is CoreException.Api -> "The server said ${e.code}: ${e.detail}"
        is CoreException.NotSignedIn -> "Sign in first."
        is CoreException.NoKey -> "This device holds no key of that group yet."
        is CoreException.Invalid -> e.reason
        is CoreException.Unknown -> "That object is not on this device."
        is CoreException.Login -> "Sign-in failed: ${e.detail}"
        is CoreException.Link -> "That is not an invite link."
        is CoreException.Crypto -> e.detail
        is CoreException.Pin -> "Device ${e.deviceId} offers another key than before."
    }

    companion object {
        /** What `register_device` is told, and `device.platform` holds. */
        const val PLATFORM = "android"

        /** The URL the browser returns to (AndroidManifest.xml). */
        const val RETURN_TO = "deappje://auth"

        fun factory(graph: AppGraph): ViewModelProvider.Factory = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T = AppViewModel(graph) as T
        }
    }
}
