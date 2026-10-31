package com.deappje.android.ui

import androidx.compose.foundation.clickable
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
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
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
import androidx.compose.ui.unit.dp

/** The groups the user is in, a new one, joining by a pasted link, and the way to the approvals. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GroupsScreen(state: UiState, model: AppViewModel, snackbar: SnackbarHostState) {
    var newGroup by remember { mutableStateOf<String?>(null) }
    var link by remember { mutableStateOf("") }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Groups") },
                actions = {
                    TextButton(onClick = { model.show(Screen.Approvals) }) {
                        Text(if (state.pending.isEmpty()) "Approvals" else "Approvals (${state.pending.size})")
                    }
                    TextButton(onClick = model::logout) { Text("Sign out") }
                },
            )
        },
        floatingActionButton = { FloatingActionButton(onClick = { newGroup = "" }) { Text("+", style = MaterialTheme.typography.headlineMedium) } },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(Modifier.padding(padding)) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(
                    value = link,
                    onValueChange = { link = it },
                    label = { Text("Invite link") },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                Spacer(Modifier.width(8.dp))
                Button(onClick = { model.join(link); link = "" }, enabled = link.isNotBlank() && !state.busy) { Text("Join") }
            }
            Text(
                "Signed in as ${state.email ?: state.userId.orEmpty()}. Device key: ${state.deviceKeyBackend}.",
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
            HorizontalDivider(Modifier.padding(top = 8.dp))
            LazyColumn {
                items(state.groups, key = { it.view.id }) { group ->
                    ListItem(
                        headlineContent = { Text(group.name ?: group.view.id) },
                        supportingContent = {
                            Text(
                                when {
                                    !group.view.holdsKeys -> "Waiting for a member to approve this device"
                                    group.view.phase == "catching_up" -> "Catching up"
                                    else -> "Up to date"
                                },
                            )
                        },
                        modifier = Modifier.clickable(enabled = group.view.holdsKeys) { model.show(Screen.Group(group.view.id)) },
                    )
                }
            }
        }
    }
    newGroup?.let { name ->
        AlertDialog(
            onDismissRequest = { newGroup = null },
            title = { Text("New group") },
            text = { OutlinedTextField(value = name, onValueChange = { newGroup = it }, label = { Text("Name") }, singleLine = true) },
            confirmButton = {
                TextButton(onClick = { model.createGroup(name); newGroup = null }, enabled = name.isNotBlank()) { Text("Create") }
            },
            dismissButton = { TextButton(onClick = { newGroup = null }) { Text("Cancel") } },
        )
    }
}
