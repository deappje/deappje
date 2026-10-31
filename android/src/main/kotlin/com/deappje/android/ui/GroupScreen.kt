package com.deappje.android.ui

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import uniffi.deappje_core.Uuid

/** A group's timeline of posts: write one, edit or delete your own, invite someone. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GroupScreen(groupId: Uuid, state: UiState, model: AppViewModel, snackbar: SnackbarHostState) {
    val group = state.groups.firstOrNull { it.view.id == groupId }
    var draft by remember { mutableStateOf("") }
    var editing by remember { mutableStateOf<Pair<Uuid, String>?>(null) }
    val context = LocalContext.current
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(group?.name ?: groupId) },
                navigationIcon = { TextButton(onClick = { model.show(Screen.Groups) }) { Text("Back") } },
                actions = { TextButton(onClick = model::invite, enabled = !state.busy) { Text("Invite") } },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(Modifier.padding(padding)) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    value = draft,
                    onValueChange = { draft = it },
                    label = { Text("Write a post") },
                    modifier = Modifier.weight(1f),
                )
                Spacer(Modifier.width(8.dp))
                Button(onClick = { model.post(draft); draft = "" }, enabled = draft.isNotBlank() && !state.busy) { Text("Post") }
            }
            LazyColumn {
                items(state.posts, key = { it.view.id }) { post ->
                    Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
                        Column(Modifier.padding(16.dp)) {
                            Text(post.text, style = MaterialTheme.typography.bodyLarge)
                            Text(
                                listOfNotNull(
                                    if (post.own) "you" else post.view.author.take(8),
                                    time(post.view.authoredAtMs),
                                    if (post.view.editedAtMs > post.view.authoredAtMs) "edited" else null,
                                    if (post.view.pending) "sending" else null,
                                ).joinToString(" · "),
                                style = MaterialTheme.typography.bodySmall,
                            )
                            if (post.own) {
                                Row {
                                    TextButton(onClick = { editing = post.view.id to post.text }) { Text("Edit") }
                                    TextButton(onClick = { model.deletePost(post.view.id) }) { Text("Delete") }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    editing?.let { (id, text) ->
        AlertDialog(
            onDismissRequest = { editing = null },
            title = { Text("Edit post") },
            text = { OutlinedTextField(value = text, onValueChange = { editing = id to it }) },
            confirmButton = { TextButton(onClick = { model.editPost(id, text); editing = null }, enabled = text.isNotBlank()) { Text("Save") } },
            dismissButton = { TextButton(onClick = { editing = null }) { Text("Cancel") } },
        )
    }
    state.inviteLink?.let { link ->
        AlertDialog(
            onDismissRequest = model::dismissInvite,
            title = { Text("Invite link") },
            text = { Text("$link\n\nShare it with the person; the part after # never reaches the server. Their device shows up under Approvals.") },
            confirmButton = {
                TextButton(onClick = {
                    context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("deappje invite", link))
                    model.dismissInvite()
                }) { Text("Copy") }
            },
            dismissButton = { TextButton(onClick = model::dismissInvite) { Text("Close") } },
        )
    }
}

private val timeFormat = DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT)

private fun time(ms: Long): String = timeFormat.format(Instant.ofEpochMilli(ms).atZone(ZoneId.systemDefault()))
