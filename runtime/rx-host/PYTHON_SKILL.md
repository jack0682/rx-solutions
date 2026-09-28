# Python SDK Host service backend

The unreleased `PYTHON_SKILL_SIMULATION` backend loads one pinned Python registration
through the product Builtin factory. Loading, inspection and initialization do not
import skill or SDK code. The existing Host gate remains the invocation owner.

```json
{
  "kind": "PYTHON_SKILL_SIMULATION",
  "registration": {
    "path": "/config/python-registration.json",
    "sha256": "<sha256-of-registration-file>"
  }
}
```

The registration uses `rx.python-skill-registration.v1` and contains installation,
Host, cell, prepared environment path/digest, input object and the exact Intent.
The Intent must be a ProgramGoal whose program references the environment manifest
and whose parameter_set references the canonical input bytes. Completion schema is
`rx.python.returned.v1`, a Python return fact, not physical completion. The single
Host binding must match installation scope, cell, Host, intent and `sim/ready`
conditions, and must be SIMULATION. Registration and environment identities are
retained at initialization; replacement after initialization is refused.

Executable selection comes from `/opt/rx/python`, not site input. The binary checks
runner/verifier hashes embedded at build time and the interpreter digest in the
release-owned `release.json`. The runtime image inventory includes this folder.
Environment preparation is described in [Python SDK preparation](../../deployment/local-skills/PYTHON_SDK.md).

`tools/test_python_host_service.py` launches the actual installed rx-hostd in an
isolated read-only container. It verifies source/image correspondence, rejects
PHYSICAL binding and changed input, initializes the Python backend, reaches
SOFTWARE_READY_UNARMED without skill/SDK import, and cleanly stops with simulation
safe-to-drop observations. This complements the actual Host gate execution tests;
it does not itself dispatch a Python operation through P.

Server-side registration commands, the full P/Executor/Host Python-skill deployment
scene, multiple programs per Host, dynamic output propagation and operator custody
reconciliation remain incomplete. The full Solutions Docker recipe includes the
helper inputs/runtime folder, but the current recorded container proof uses the
focused RuntimeSkillValidation image. Neither establishes physical qualification.
