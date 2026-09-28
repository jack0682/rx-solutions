# Recoverable Host binding commit (unreleased)

This local maintenance transition applies a prepared, normally stopped Host binding
change without resetting delivery/evidence journals or erasing earlier native data.
Current commit support is SIMULATION with unchanged cell cohort and scope topology.
A plan artifact alone is not execution authority, and this command never opens an
adapter or restores Arm/qualification.

```text
rx-hostd prepare-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG REQUEST_ID
rx-hostd commit-binding-change PLAN CURRENT_CONFIG PROPOSED_CONFIG REQUEST_ID
rx-hostd lookup-binding-commit CURRENT_OR_PROPOSED_CONFIG REQUEST_ID
```

The commit records its original fingerprint before creating native metadata in a
new `native-generations/REQUEST_ID` directory. It then records the proposed descriptor,
publishes that descriptor atomically, and marks the transaction committed. The
startup configuration files themselves are not overwritten; start with the exact
proposed configuration after commit. Old startup identity is rejected.

Pending commit states block startup and cancellation. Retry the original commit,
with the original material, after interruption. Reusing its ID with changed inputs
is refused. An already committed request cannot roll back a later installed state.
Lookup distinguishes the stored receipt from whether it is currently installed.
Recovery still requires original configuration/package material and valid trust;
resolution for changed or revoked deployment material remains separate work.

The old native generation remains available as historical evidence. A missing or
changed generation, different journal identity, stale normal-stop state, or unresolved
native work is not repaired by initializing new authority storage. Publication
interruption is recoverable without replaying a device operation because metadata
initialization is passive.

A durable barrier rejects Arm after commit until the authenticated P configuration
request names the exact committed target. That request remains subject to existing
Host configuration checks and does not itself grant qualification. Subsequent Arm
still needs the current qualification/context. Wrong target configuration cannot
consume this barrier.

Tests kill the actual maintenance child process after intent persistence, native
publication and descriptor publication, then recover the same request. They compare
journal IDs, retained old native data, no Arm restoration, immutable request handling
and rejection of a corrupted staged marker. These are local Host/service tests with
FILE_SIMULATION, not an installed signed-Python-package swap or a P deployment proof.

Compatibility: this extends internal maintenance records with COMMITTING/COMMITTED
states and a native-generation descriptor field. Use this build for recovery;
older binaries are not supported after these records are written. The local commit uses internal records. The optional Host configuration binding
now uses source revision 3 to report current-boot commit metadata and both baseline journals; callers must
negotiate that exact binding hash. P still blocks Host replacement until its fresh-result
confirmation contract is implemented; do not remove that guard as a shortcut.
