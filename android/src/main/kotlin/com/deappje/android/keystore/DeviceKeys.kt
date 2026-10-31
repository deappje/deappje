package com.deappje.android.keystore

import android.os.Build
import com.deappje.android.shell.SoftwareDeviceKey
import uniffi.deappje_core.DeviceKey
import uniffi.deappje_core.Random
import uniffi.deappje_core.Secrets

/** The device key in use and where it lives, for the screen that says so. */
class DeviceKeyChoice(val key: DeviceKey, val backend: String)

/**
 * Which device key this phone has. A key, once minted, is kept for good
 * (the server refuses a changed key under a known device id), so what is
 * kept wins over what is possible: a software key in the secrets, then a
 * Keystore key under its alias, then a new Keystore key on API 33+, and a
 * new software key last.
 */
object DeviceKeys {
    const val KEYSTORE = "Android Keystore (X25519 inside the Keystore)"
    const val SOFTWARE = "software X25519 (secret sealed under the Keystore)"

    fun open(secrets: Secrets, random: Random): DeviceKeyChoice {
        SoftwareDeviceKey.kept(secrets)?.let { return DeviceKeyChoice(it, SOFTWARE) }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            (KeystoreDeviceKey.kept() ?: KeystoreDeviceKey.mint())?.let { return DeviceKeyChoice(it, KEYSTORE) }
        }
        return DeviceKeyChoice(SoftwareDeviceKey.keptOrMinted(secrets, random), SOFTWARE)
    }
}
