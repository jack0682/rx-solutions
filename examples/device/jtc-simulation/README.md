# ROS JTC authoring example — SIMULATION

Authoring input that selects catalog SIM-JTC-6DOF/arm_controller. calibration.bin/tool.bin contain TEST ONLY strings; they are not physical measurements or evidence of mechanical compatibility. Coordinates, joint targets, and domain171 are for simulation tests. Package signatures, operational trust, and operating qualification are not included.

```text
rx-device-package template-digest template.json
rx-device-package assemble template.json site.json recipe.json /absolute/new/candidate
```

site.json references the template's semantic digest and the SHA-256 of artifact bytes. After changing the template, obtain a new digest with the tool. Assembly does not connect to ROS or devices. Publication requires an external signature and a separate policy that verifies the relevant publisher, authority, and assets.

The current product Host has no connected JTC execution provider. Creating a package or performing init cannot drive a device. Follow the [JTC package structure and Host boundary](../../../runtime/rx-host/JTC_PACKAGE.md).
