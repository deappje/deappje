"""The NDK for the Android libraries of the core, only when there is one.

rules_android_ndk's own extension fails the fetch when ANDROID_NDK_HOME is
unset, and a registered toolchain is loaded by every toolchain resolution,
so the server's `bazel test //...` on a machine without an NDK would fail
on it. This extension reads the variable itself: with an NDK it is
rules_android_ndk's repository; without one it is an empty repository
whose `:all` registers no toolchain. Every target that needs the NDK is
behind --//:apps anyway (docs/01-deappje/05-deployment.md, "The apps").
"""

load("@rules_android_ndk//:rules.bzl", "android_ndk_repository")

def _empty_ndk_impl(repo_ctx):
    repo_ctx.file("BUILD.bazel", "# No NDK: ANDROID_NDK_HOME was unset when this repository was fetched.\n")
    repo_ctx.file("WORKSPACE", "")

_empty_ndk = repository_rule(
    implementation = _empty_ndk_impl,
    doc = "An NDK repository with nothing in it.",
)

def _android_ndk_impl(module_ctx):
    api_level = 0
    for module in module_ctx.modules:
        if not module.is_root:
            continue
        for tag in module.tags.configure:
            api_level = tag.api_level
    if module_ctx.getenv("ANDROID_NDK_HOME"):
        android_ndk_repository(name = "androidndk", api_level = api_level)
    else:
        _empty_ndk(name = "androidndk")
    return module_ctx.extension_metadata(reproducible = True)

android_ndk = module_extension(
    implementation = _android_ndk_impl,
    doc = "rules_android_ndk's repository when ANDROID_NDK_HOME is set, an empty one otherwise.",
    tag_classes = {
        "configure": tag_class(
            attrs = {
                "api_level": attr.int(doc = "The Android API level the NDK's sysroot targets."),
            },
        ),
    },
)
