# Laser heat-treatment material exchange: site workflow draft

The user confirmed the existing laser heat-treatment process. This draft uses the
CNC-labelled exchange image as a sequencing reference: prepare the next raw
material, align it, wait at the machine, remove the finished part, switch end
effector, clean/insert/seat/clamp/release, retreat, close/start and deposit the
finished part. It replaces generic delivery examples as the process reference.
It does not fix a robot model or claim that a ROBOTIS adapter can already perform it.

The incoming material and outgoing finished part are distinct roles. Prefetch and
alignment occur before waiting for machining completion. The reference image's stopped-machine
interval spans the exchange; machining can continue while the robot deposits the
finished part and prepares the next one. The source describes one steady-state
exchange; the Repeat count is one for authoring. First loading of an empty machine,
last-part removal, quantities and interrupted-cycle recovery are not inferred.

`source.json` uses the existing ProcessSource model and reusable subworkflows.
`boundary.json` records missing facts and distinguishes added observation gates from
steps explicitly shown in the image. All Wait deadlines are intentionally zero:
the existing validator reports WAIT_LIMIT until real reviewed deadlines are supplied.
The example therefore remains an incomplete authoring draft. No PLC address, motion,
TCP, force, workpiece dimension, runtime capability or physical qualification is invented.

Observe these authored orderings when connecting actual implementations:

- Grip the finished part before unclamping, then extract it.
- Insert and check seating before clamping and releasing the incoming material.
- Retreat and confirm the relevant door-clear condition before closing and starting.

These observations are proposed verification gates, not proof of a safe physical
procedure. No executable bindings are supplied. Command acceptance, actual operation
completion, part custody and resource handover must be verified separately. A lost
response must query/reconcile the original operation instead of automatically issuing
another clamp, release or cycle-start command.


## Product and tooling recipes

`tooling-reference.json` records the second supplied drawing. The interpretation is
five part families (ECC_51, ECC_99, CVR_F, FLANGE_L, FLANGE_R) in two forms (14/17):
ten product variants. CVR_F front/back are distinct setups, not additional product
identities. This gives twelve setup rows and eight jig variants. The form labels
are identifiers, not asserted millimetre dimensions.

| Part / face | Jig family (both forms) | Finger visual grouping |
|---|---|---|
| ECC_51 / ECC_99 | PART1 | Blue |
| CVR_F front | PART2 | Blue |
| CVR_F back | PART3 | Magenta |
| FLANGE_L | PART4 | Cyan |
| FLANGE_R | PART2 | Magenta |

Finger colors are observations of the illustration, not verified tool part numbers.
Actual finger identity, TCP/calibration, dimensions, grip force, poses, process recipe
and installed-tool verification remain unset. The same workflow structure is reused
with explicit product/form/face setup records; the table does not authorize execution.
The initial/final-cycle and physical-interface gaps in boundary.json remain open.

## Single and dual grippers

Select an explicit profile from `gripper-profiles.json`. Grasp slots mean independent
workpiece-holding channels; the number of fingers on one gripper is not a slot count.

- `dual-gripper.source.json`: prefetch/align incoming material, wait, extract the
  finished part into the other slot, switch end effector, insert incoming material,
  restart processing and deposit the finished part.
- `single-gripper.source.json`: wait, extract and deposit the finished part, then
  pick/align the incoming material, insert it and restart processing. There is no
  end-effector switch or simultaneous incoming/finished grip assumption.
- `single-buffered.source.json`: optional prefetch through an explicitly reviewed
  temporary support resource. Park and confirm support of the incoming material
  before handling the finished part, then retrieve it for insertion. This route is
  not available until the buffer and its custody conditions are configured.

`source.json` is the dual-mode draft retained as an explicit example entry. It is not
an automatic profile selector. All profiles reuse the product/form/face tooling
reference, but that reference does not establish that a particular installed single
or dual tool fits each part. Each profile still has unconfigured observation deadlines
and no executable device bindings.

## Dedicated rotary alignment equipment

The user confirmed that a separate rotary alignment device is used in both single
and dual configurations. `orient-next` therefore delegates detection/rotation to
`alignment-station/*` operations and checks proposed readiness, seating, alignment
completion and stopped-state observations before regripping. It does not substitute
a robot wrist rotation or the processing machine's spindle for this device.

`equipment-reference.json` keeps the dedicated equipment role separate from the
robot and processing machine. Its actual interface, detector ownership, grip-release
procedure, part support, rotation limits and timeouts are still unspecified. No
assumption is made that the robot may release or hold the part while it rotates.
Dual mode retains this alignment phase during prefetch before waiting at the laser
machine; single mode performs it after the finished part has been deposited unless
the explicitly configured temporary-buffer profile is selected.
