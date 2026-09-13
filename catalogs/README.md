# Device support catalog

`device-support.v1.json` is a vendor-independent device declaration format. Connecting an external device requires a repository URL, an immutable Git commit, reference-file SHA-256 values, and separate site commissioning evidence. No particular company or model count is imposed as a mandatory list for the common core.

The current default catalog contains only **4 simulation declarations** newly written in `fixtures/controllers.v1.json`. The 6-axis and 7-axis position JTC profiles exercise allowed paths; leader and impedance profiles test rejection at the support boundary. A gripper-only action is also not admitted as JTC. `SIMULATION_FIXTURE` does not mean a physical model, observed vendor source, or completed device validation.

`rx-solution-catalog` validates duplicates, Git revisions, fixture provenance, controller declarations, and required commissioning inputs. It also compares the actual hashes of bundled fixture sources and each controller declaration. `SOURCE_OBSERVED` and `SIMULATION_FIXTURE` represent different evidence and cannot be converted by renaming them. The Host rejects using a simulation fixture as a physical-environment profile.

```sh
python3 tools/check_device_catalog_sources.py
./tools/cargo test -p rx-solution-catalog
```

Use `--catalog PATH --source-root PATH` to validate an external catalog. The command checks source identity and does not start ROS controllers. Native authority, calibration, startup effects, completion evidence, and handover conditions must be validated separately when adding a physical device. Earlier support tables and original test records remain in Git history; their results are not inherited as validation results for this new catalog.
