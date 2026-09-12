use super::*;
mod production_expiry;

#[test]
fn cleanup_confirmation_distinguishes_no_spawn_and_local_close_from_uncertainty() {
    let lifecycle = PlannerLifecycle::default();
    assert_eq!(lifecycle.result(), PlannerCleanup::NotStarted);
    assert!(PlannerCleanup::NotStarted.is_confirmed());
    assert!(PlannerCleanup::Confirmed.is_confirmed());
    assert!(!PlannerCleanup::Unconfirmed.is_confirmed());
    assert_eq!(
        serde_json::to_string(&PlannerCleanup::Unconfirmed).unwrap(),
        "\"UNCONFIRMED\""
    );
}

#[tokio::test]
async fn each_serial_planner_requires_its_own_confirmed_cleanup() {
    let mut lifecycle = PlannerLifecycle::default();
    for visit in 1..=2 {
        lifecycle.starting();
        assert_eq!(lifecycle.result(), PlannerCleanup::Unconfirmed);
        let closed = lifecycle
            .finish(async { Ok::<_, engine_process::Error>(visit) })
            .await
            .unwrap();
        assert_eq!(closed, visit);
        assert_eq!(lifecycle.result(), PlannerCleanup::Confirmed);
    }
}

#[tokio::test]
async fn failed_close_is_sticky_after_a_later_successful_cleanup() {
    for error in [
        engine_process::Error::Timeout,
        engine_process::Error::Invalid("planner close failed".into()),
        engine_process::Error::Io(std::io::Error::other("pipe closed")),
    ] {
        let mut lifecycle = PlannerLifecycle::default();
        lifecycle.starting();
        assert!(
            lifecycle
                .finish(async { Err::<(), _>(error) })
                .await
                .is_err()
        );
        assert_eq!(lifecycle.result(), PlannerCleanup::Unconfirmed);

        // The same tracker serves normal stop and serial retire. No later successful
        // cleanup or overwritten diagnostic can erase the first unconfirmed result.
        lifecycle.starting();
        lifecycle
            .finish(async { Ok::<_, engine_process::Error>(()) })
            .await
            .unwrap();
        assert_eq!(lifecycle.result(), PlannerCleanup::Unconfirmed);
    }
}

#[tokio::test]
async fn cancelled_serial_retirement_cannot_become_confirmation_for_an_empty_engine_slot() {
    let mut lifecycle = PlannerLifecycle::default();
    lifecycle.starting();
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    let mut cleanup = Box::pin(lifecycle.finish(async {
        let _ = cancelled.await;
        Ok::<_, engine_process::Error>(())
    }));
    // Poll cleanup into its await, then cancel it just as shutdown.changed() can
    // cancel cycle() after the engine was taken for serial retirement.
    tokio::select! {
        biased;
        _ = &mut cleanup => panic!("cleanup unexpectedly completed"),
        _ = std::future::ready(()) => {}
    }
    drop(cleanup);
    assert_eq!(lifecycle.result(), PlannerCleanup::Unconfirmed);
    assert!(
        cancel.send(()).is_err(),
        "retirement future must be dropped"
    );

    // Cancellation is latched even if another cleanup success is later presented.
    lifecycle
        .finish(async { Ok::<_, engine_process::Error>(()) })
        .await
        .unwrap();
    assert_eq!(lifecycle.result(), PlannerCleanup::Unconfirmed);
}

#[tokio::test]
async fn spawn_attempts_are_uncertain_until_cleanup_and_failures_remain_latched() {
    let mut interrupted = PlannerLifecycle::default();
    interrupted.starting();
    // A cancelled factory future has not returned an EngineProcess handle. The
    // attempted spawn alone must prevent NotStarted or an automatic next spawn.
    assert_eq!(interrupted.result(), PlannerCleanup::Unconfirmed);
    assert!(!interrupted.result().is_confirmed());

    let mut failed = PlannerLifecycle::default();
    failed.starting();
    failed.spawn_failed();
    failed
        .finish(async { Ok::<_, engine_process::Error>(()) })
        .await
        .unwrap();
    assert_eq!(failed.result(), PlannerCleanup::Unconfirmed);
}
