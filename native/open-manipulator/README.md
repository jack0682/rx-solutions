# OpenMANIPULATOR L3 variant

The unchanged G9 gate was applied to real s6-overlay 3.2.1.0 before this variant
was authored. It blocked in-container service restarts, but its evidence remained
bound to `ai-worker/2.2.7/l3-simulation`; therefore unchanged reuse was rejected.

G10 is the `CATALOG_BOUND_SERVICE_IDENTITY` variant. The same guard now accepts
only two compiled service identities, and each release catalog recipe supplies
its own fixed identity and state root. For OpenMANIPULATOR the identity is
`open-manipulator/5.1.2/l3-simulation`. This is a variant, not a claim that the
G9 artifact was reusable unchanged.

The simulation runs no ROS service, camera, DYNAMIXEL, MoveIt or Gazebo.
RealSense, architecture selection, mode dependencies and maintenance handoff are
separate named denials. Physical qualification and ai_sapiens integration remain
unfinished; the ROBOTIS bundle is not complete.
