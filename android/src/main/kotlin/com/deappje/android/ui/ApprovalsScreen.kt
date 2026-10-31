package com.deappje.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * The key requests this device may approve (docs/01-deappje/02-device.md):
 * the invite vouches for a joiner, or the person compares the code the
 * requesting device shows. There is no silent path.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ApprovalsScreen(state: UiState, model: AppViewModel, snackbar: SnackbarHostState) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Approvals") },
                navigationIcon = { TextButton(onClick = { model.show(Screen.Groups) }) { Text("Back") } },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        if (state.pending.isEmpty()) {
            Text("No device is waiting.", modifier = Modifier.padding(padding).padding(16.dp))
            return@Scaffold
        }
        LazyColumn(Modifier.padding(padding)) {
            items(state.pending, key = { it.requestId }) { request ->
                val group = state.groups.firstOrNull { it.view.id == request.groupId }
                Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Column(Modifier.padding(16.dp)) {
                        Text("${request.platform} device of ${request.userId}", style = MaterialTheme.typography.titleMedium)
                        Text("Group: ${group?.name ?: request.groupId}")
                        Text("Code: ${request.fingerprint}", style = MaterialTheme.typography.titleLarge)
                        Text(
                            when (request.inviteVerified) {
                                true -> "Came through your invite link: the link vouches for it."
                                false -> "Claims your invite link but the tag does not match. Do not approve."
                                null -> "No invite: compare the code with the one on that device's screen before approving."
                            },
                            style = MaterialTheme.typography.bodyMedium,
                        )
                        Button(
                            onClick = { model.approve(request.requestId) },
                            enabled = request.inviteVerified != false && !state.busy,
                            modifier = Modifier.padding(top = 8.dp),
                        ) { Text("Approve") }
                    }
                }
            }
        }
    }
}
