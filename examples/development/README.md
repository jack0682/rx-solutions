# Local development materials

`cell-demo.json` is an **unvalidated simulated cell configuration** for connecting the UI and API. It is not a physical device or process package, and its SHA-256 values are explicit placeholders rather than hashes of existing artifacts. It supplies no Host registration, device driver, completion evidence, or operating qualification.

Registration through the local service's `POST /api/v1/cells` leaves it awaiting validation. Do not use it as product package input or configuration for the first laser site. It is simulation test data unrelated to the Nemo Engineering site's Q03UDVCPU, robot, gripper, fixture, or signal table.
