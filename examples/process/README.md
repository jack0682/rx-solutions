# Material supply process example

Source and simulation bindings for: open door → place material → close chuck → robot exit → close door. This does not mean that the execution sequence or interlocks of any physical site have been validated.

`material-supply.source.json` contains the process structure; `material-supply.bindings.json` contains explicit simulation targets. Program/parameter artifacts are placeholders. They contain no Q03UDVCPU addresses, actual robot motion, TCP, calibration, sensor judgments, or material support information and must not be used for operation.

The example can be compiled with `rx-process-compile`. This compilation checks files, structure, and resource conflicts; execution of RX custom BT nodes, device qualification, and physical acceptance remain future work.
