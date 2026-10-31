package com.deappje.android.ui

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.browser.customtabs.CustomTabsIntent
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import com.deappje.android.DeappjeApplication
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The one activity. It owns what only the platform does: the Custom Tab the
 * sign-in opens, the deappje://auth return that carries the code, and the
 * foreground: a sync when the app comes forward and every 30 seconds while
 * it stays there (core/AGENTS.md, "Wake-ups"), with the approvals list
 * re-read each time since no sync lists it.
 */
class MainActivity : ComponentActivity() {
    private val model: AppViewModel by viewModels { AppViewModel.factory((application as DeappjeApplication).graph) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        returned(intent)
        setContent {
            DeappjeTheme {
                DeappjeApp(model, onSignIn = { model.signIn(::openInBrowser) })
            }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                while (true) {
                    model.refresh()
                    delay(SYNC_INTERVAL_MS)
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        returned(intent)
    }

    /** The browser's way back: deappje://auth?code=... */
    private fun returned(intent: Intent?) {
        val data = intent?.data ?: return
        if (data.scheme == "deappje" && data.host == "auth") {
            data.getQueryParameter("code")?.let(model::finishLogin)
        }
    }

    private fun openInBrowser(url: Uri) {
        CustomTabsIntent.Builder().build().launchUrl(this, url)
    }

    private companion object {
        const val SYNC_INTERVAL_MS = 30_000L
    }
}
