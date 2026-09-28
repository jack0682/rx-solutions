# Preparing a Python skill with SDK wheels

This unreleased preparation command installs `skill.py`, `skill.json` and explicitly
supplied SDK wheels in a new local environment. It does not register the result in
P or permit Host/device execution. The runtime bridge remains work in progress.

```sh
rx skill prepare-environment ./my-skill --wheel ./vendor_sdk-1.0.0-py3-none-any.whl --output ./prepared-skill
rx skill verify-environment ./prepared-skill --digest DIGEST_FROM_PREPARATION
```

The skill defines `main(inputs)`. Supply compatible wheels for every dependency;
there is no network dependency resolution, source build or runtime package install.
Pip chooses compatibility for the current interpreter/platform. Native SDK system
libraries and the complete base Python runtime must be pinned by the eventual
release image; this helper does not verify them. Its result therefore explicitly
sets `base_runtime_release_verified: false` and `execution_authorized: false`.
Missing SDK dependencies are not proven absent merely by successful preparation.

The wheel bytes, skill files and resulting environment inventory are recorded under
an externally retainable digest. Verification rejects changed files, executable
bits, symlinks, the base interpreter binary or a moved environment. Keep the digest
outside that directory when using it as an integrity reference. A digest computed
from whatever directory currently exists is not a registration identity.

Preparation does not import the skill or SDK. Wheels containing startup `.pth`,
site/user customization hooks or escaping paths are rejected. This is a trusted
author preparation tool, not a hostile-code sandbox or physical qualification.
Existing output is never replaced; interrupted preparation has no ready manifest.
The environment is bound to its original path and platform and is not a portable
release archive. See the standard [wheel format](https://packaging.python.org/en/latest/specifications/binary-distribution-format/)
and [Python virtual environments](https://docs.python.org/3/library/venv.html).

`tools/test_python_environment.py` invokes the actual CLI, installs a separate
fixture SDK wheel offline, explicitly executes a test skill importing that SDK,
and checks tamper/no-overwrite/startup-hook rejection. The test execution is separate
from preparation and has no device or P/Host execution authority. Next integration
must bind this environment to registered Host ProgramGoal execution and original
request recovery; preparation alone must not be presented as that integration.
