package com.deappje.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle

/** The screens, one at a time by [UiState.screen]; notices as snackbars. */
@Composable
fun DeappjeApp(model: AppViewModel, onSignIn: () -> Unit) {
    val state by model.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(state.notice) {
        val notice = state.notice ?: return@LaunchedEffect
        snackbar.showSnackbar(notice)
        model.dismissNotice()
    }
    when {
        state.restoring -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        !state.signedIn -> SignInScreen(state, onSignIn, snackbar)
        else -> {
            val screen = state.screen
            BackHandler(enabled = screen != Screen.Groups) { model.show(Screen.Groups) }
            when (screen) {
                Screen.Groups -> GroupsScreen(state, model, snackbar)
                Screen.Approvals -> ApprovalsScreen(state, model, snackbar)
                is Screen.Group -> GroupScreen(screen.id, state, model, snackbar)
            }
        }
    }
    SnackbarHost(snackbar)
}
