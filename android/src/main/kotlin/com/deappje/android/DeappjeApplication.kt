package com.deappje.android

import android.app.Application
import android.content.Context
import com.deappje.android.keystore.DeviceKeys
import com.deappje.android.keystore.KeystoreKeys
import com.deappje.android.push.NoPush
import com.deappje.android.push.PushTokenSource
import com.deappje.android.shell.EncryptedFileSecrets
import com.deappje.android.shell.FileStorage
import com.deappje.android.shell.OkHttpTransport
import com.deappje.android.shell.SecureRandomSource
import com.deappje.android.shell.SystemClock
import java.io.File
import uniffi.deappje_core.Core

/** The process: one [AppGraph], and in it the one [Core] every screen uses. */
class DeappjeApplication : Application() {
    lateinit var graph: AppGraph
        private set

    override fun onCreate() {
        super.onCreate()
        graph = AppGraph.open(this)
    }
}

/**
 * The core over this phone's seams (docs/01-deappje/01-architecture.md, "The
 * shells"). The store and the secrets live under the no-backup files
 * directory: they are this device's, whose key never changes, and must not
 * follow a backup onto another.
 */
class AppGraph(val core: Core, val deviceKeyBackend: String, val push: PushTokenSource) {
    companion object {
        fun open(context: Context): AppGraph {
            val random = SecureRandomSource()
            val secrets = EncryptedFileSecrets(File(context.noBackupFilesDir, "secrets"), KeystoreKeys.secretsKey())
            val device = DeviceKeys.open(secrets, random)
            val storage = FileStorage(File(context.noBackupFilesDir, "store"))
            val core = Core(OkHttpTransport(), storage, secrets, device.key, random, SystemClock(), Environment.BASE_URL)
            return AppGraph(core, device.backend, NoPush)
        }
    }
}
