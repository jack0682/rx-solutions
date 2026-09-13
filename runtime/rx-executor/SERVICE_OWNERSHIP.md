# Recovering executor ownership after Run exit

`RunService::run_owned()` returns `ServiceExit<R, F>` after completing the existing run loop and stop/pause handling. `report` is the existing `Report`, `worker` is the `Worker<R>` used, and `factory` is the planner factory used. `Worker::into_parts()` returns that Worker's `Client` and `Journal<R>` without reopening them. The Client's authenticated session and shared runtime boot/sequence/checked-time validation state remain intact.

Existing `RunService::run()` is a wrapper returning only the Report from `run_owned()`. Existing run-only CLI output, stop/pause requests/observations/storage, communication grace and timeout flows are preserved.

`ServiceExit::planner_cleanup` separately reports local planner cleanup results.

- `NOT_STARTED`: this RunService did not attempt planner creation.
- `CONFIRMED`: all created planners completed existing CLOSE response validation and child-exit confirmation.
- `UNCONFIRMED`: spawn or close/serial retire failed or was cancelled, leaving child ownership/exit unconfirmed.

`is_confirmed()` is true only for NOT_STARTED and CONFIRMED. This is a prerequisite for transitioning to the next Run; it does not replace actual P COMPLETED, exact assignment journal binding, current session and fresh-state validation. It is also not proof of physical Host/controller stop or resource handover.

Unconfirmed state is recorded before spawn await. Spawn failure does not establish that no child existed; cancellation of await does not revert to NOT_STARTED even without a local handle. Normal-stop close and serial-material-completion retire use the same cleanup tracker. CLOSE failure or in-progress cancellation remains sticky UNCONFIRMED; later last_error string changes, an empty engine slot or another successful close do not clear it. A cycle with no handle after cancellation creates no new planner and converges to PlannerFault stop.

Dedicated unit tests cover cleanup required for each successive serial planner, persistent close errors, cancellation of an in-progress cleanup future and uncertain spawn state. Actual P state, ownership recovery and next-Run transitions are verified separately in resident cell service integration tests.
