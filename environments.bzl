"""The platform hostnames the apps talk to (docs/01-deappje/05-deployment.md,
"The apps", "Base URL"): the preview and live environments of the project
as `get_project` reports them. `//:environment` picks one at build time
(`preview` by default; apps.yaml passes `live` on main). A custom domain
later changes nothing an installed app relies on, since the platform
hostnames keep serving. The web client needs none of this: it is served
from the environment itself and talks to its own origin.
"""

ENVIRONMENTS = {
    "preview": "https://abandon-above-toss.preview.basable.com",
    # The live environment has no deployment yet (get_project shows no
    # live_url); this is the platform's hostname for it.
    "live": "https://abandon-above-toss.basable.com",
}

def base_url(environment):
    """The origin of an environment, with no trailing slash."""
    if environment not in ENVIRONMENTS:
        fail("unknown environment %r; one of %s" % (environment, sorted(ENVIRONMENTS)))
    return ENVIRONMENTS[environment]
