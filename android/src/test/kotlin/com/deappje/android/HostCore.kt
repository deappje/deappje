package com.deappje.android

import java.io.File

/**
 * The host build of the core for the tests (docs/01-deappje/03-testing.md,
 * "App shells"): `//core:core_host` is a data dependency, so its
 * `libdeappje_core.{dylib,so}` sits under `core/` in the runfiles, and the
 * bindings load it through JNA by that name once the directory is on JNA's
 * library path.
 */
object HostCore {
    private val loaded: Boolean by lazy {
        val runfiles = System.getenv("TEST_SRCDIR")
        if (runfiles != null) {
            val workspace = System.getenv("TEST_WORKSPACE") ?: "_main"
            System.setProperty("jna.library.path", File(runfiles, "$workspace/core").absolutePath)
        }
        true
    }

    fun load() {
        check(loaded)
    }
}

/** A test that may reach the core: the library path is set before anything else. */
abstract class HostCoreTest {
    init {
        HostCore.load()
    }
}
