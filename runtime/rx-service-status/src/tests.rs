use super::*;
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    sync::Mutex,
};

struct TestClock(Mutex<TimePoint>);
impl Clock for TestClock {
    fn now(&self) -> Result<TimePoint> {
        Ok(self.0.lock().unwrap().clone())
    }
}
impl TestClock {
    fn set(&self, now: TimePoint) {
        *self.0.lock().unwrap() = now;
    }
}
fn id(value: u32) -> Id {
    Id::new(format!("{value:08x}-0000-4000-8000-000000000000")).unwrap()
}
fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}
fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
fn time(ticks: u64) -> TimePoint {
    TimePoint {
        clock_id: "test/guarded-status".into(),
        ticks_ns: Counter(ticks),
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    binding: GuardedStatusBinding,
    clock: Arc<TestClock>,
    instance: Id,
    pid: u32,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(root.path()).unwrap().join("status.json");
        Self {
            _root: root,
            binding: GuardedStatusBinding::Host {
                path,
                installation: id(1),
                host: name("host/a"),
                installation_identity: digest(2),
            },
            clock: Arc::new(TestClock(Mutex::new(time(5_000_000_000)))),
            instance: id(3),
            pid: 42,
        }
    }
    fn reporter(&self) -> Reporter {
        Reporter::with_clock(
            self.binding.path().into(),
            self.binding.scope(),
            self.instance.clone(),
            self.pid,
            self.clock.clone(),
        )
        .unwrap()
    }
    fn read(&self) -> Result<Option<GuardedObservation>> {
        Reader::with_clock(self.clock.clone()).read(&self.binding, &self.instance, self.pid)
    }
    fn rewrite(&self, change: impl FnOnce(&mut Envelope)) {
        let bytes = fs::read(self.binding.path()).unwrap();
        let mut envelope: Envelope = canonical::decode_json(&bytes).unwrap();
        change(&mut envelope);
        fs::write(self.binding.path(), canonical::bytes(&envelope).unwrap()).unwrap();
    }
}

#[test]
fn lifecycle_publication_preserves_exact_source_time_and_owner() {
    let fixture = Fixture::new();
    assert!(fixture.read().unwrap().is_none());
    let mut reporter = fixture.reporter();
    assert_eq!(reporter.scope(), &fixture.binding.scope());
    assert_eq!(reporter.instance(), &fixture.instance);
    assert_eq!(reporter.pid(), fixture.pid);
    assert_eq!(reporter.path(), fixture.binding.path());
    for (index, state) in [
        GuardedState::Starting,
        GuardedState::Ready,
        GuardedState::Stopping,
        GuardedState::Stopped {
            reconciliation_required: true,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let sent = reporter.publish(state).unwrap();
        assert_eq!(sent.sequence, Counter(index as u64 + 1));
        assert_eq!(fixture.read().unwrap(), Some(sent.clone()));
        // A repeated read cannot refresh the source observation.
        fixture.clock.set(time(sent.observed_at.ticks_ns.0 + 100));
        assert_eq!(fixture.read().unwrap(), Some(sent));
    }
}

#[test]
fn host_role_installation_identity_instance_and_pid_are_exact() {
    let fixture = Fixture::new();
    fixture.reporter().publish(GuardedState::Ready).unwrap();
    let reader = Reader::with_clock(fixture.clock.clone());
    assert!(reader.read(&fixture.binding, &id(99), fixture.pid).is_err());
    assert!(
        reader
            .read(&fixture.binding, &fixture.instance, fixture.pid + 1)
            .is_err()
    );
    assert!(reader.read(&fixture.binding, &fixture.instance, 0).is_err());
    let path = fixture.binding.path().to_owned();
    for binding in [
        GuardedStatusBinding::Host {
            path: path.clone(),
            installation: id(99),
            host: name("host/a"),
            installation_identity: digest(2),
        },
        GuardedStatusBinding::Host {
            path: path.clone(),
            installation: id(1),
            host: name("host/b"),
            installation_identity: digest(2),
        },
        GuardedStatusBinding::Host {
            path: path.clone(),
            installation: id(1),
            host: name("host/a"),
            installation_identity: digest(99),
        },
        GuardedStatusBinding::Executor {
            path,
            installation: id(1),
            cell: name("cell/a"),
            service_journal: id(4),
            configuration_digest: digest(2),
        },
    ] {
        assert!(
            reader
                .read(&binding, &fixture.instance, fixture.pid)
                .is_err()
        );
    }
}

#[test]
fn executor_cell_journal_and_configuration_are_exact_scope() {
    let mut fixture = Fixture::new();
    fixture.binding = GuardedStatusBinding::Executor {
        path: fixture.binding.path().into(),
        installation: id(1),
        cell: name("cell/a"),
        service_journal: id(4),
        configuration_digest: digest(5),
    };
    fixture
        .reporter()
        .publish(GuardedState::Stopped {
            reconciliation_required: false,
        })
        .unwrap();
    assert!(matches!(
        fixture.read().unwrap().unwrap().state,
        GuardedState::Stopped {
            reconciliation_required: false
        }
    ));
    for change in [
        (|v: &mut GuardedStatusBinding| {
            if let GuardedStatusBinding::Executor {
                service_journal, ..
            } = v
            {
                *service_journal = id(99);
            }
        }) as fn(&mut GuardedStatusBinding),
        |v| {
            if let GuardedStatusBinding::Executor { cell, .. } = v {
                *cell = name("cell/b");
            }
        },
        |v| {
            if let GuardedStatusBinding::Executor {
                configuration_digest,
                ..
            } = v
            {
                *configuration_digest = digest(99);
            }
        },
    ] {
        let mut binding = fixture.binding.clone();
        change(&mut binding);
        assert!(
            Reader::with_clock(fixture.clock.clone())
                .read(&binding, &fixture.instance, fixture.pid)
                .is_err()
        );
    }
}

#[test]
fn stale_future_and_other_clock_observations_are_rejected_without_refresh() {
    let fixture = Fixture::new();
    let observation = fixture.reporter().publish(GuardedState::Ready).unwrap();
    fixture
        .clock
        .set(time(observation.observed_at.ticks_ns.0 + MAX_AGE_NS));
    assert_eq!(fixture.read().unwrap(), Some(observation.clone()));
    for now in [
        time(observation.observed_at.ticks_ns.0 + MAX_AGE_NS + 1),
        time(observation.observed_at.ticks_ns.0 - 1),
        TimePoint {
            clock_id: "test/other-clock".into(),
            ticks_ns: observation.observed_at.ticks_ns,
        },
    ] {
        fixture.clock.set(now);
        assert!(fixture.read().is_err());
    }
}

#[test]
fn schema_zero_sequence_and_strict_payload_shape_are_enforced() {
    let fixture = Fixture::new();
    fixture.reporter().publish(GuardedState::Ready).unwrap();
    let original = fs::read(fixture.binding.path()).unwrap();
    for change in [
        (|v: &mut Envelope| v.sequence = Counter(0)) as fn(&mut Envelope),
        |v| v.schema = name("rx.other-status.v1"),
        |v| v.pid = 0,
        |v| v.observed_at.clock_id.clear(),
    ] {
        fs::write(fixture.binding.path(), &original).unwrap();
        fixture.rewrite(change);
        assert!(fixture.read().is_err());
    }
    let text = String::from_utf8(original).unwrap();
    for bytes in [
        Vec::new(),
        b"{}".to_vec(),
        format!("{{\"sequence\":\"1\",{}", &text[1..]).into_bytes(),
        format!("{{\"admission_allowed\":true,{}", &text[1..]).into_bytes(),
        text.replace("\"sequence\":\"1\"", "\"sequence\":1")
            .into_bytes(),
        vec![b' '; MAX_BYTES + 1],
    ] {
        fs::write(fixture.binding.path(), bytes).unwrap();
        assert!(fixture.read().is_err());
    }
}

#[test]
fn reader_preserves_same_sequence_content_changes_for_supervisor_comparison() {
    let fixture = Fixture::new();
    let first = fixture.reporter().publish(GuardedState::Ready).unwrap();
    fixture.rewrite(|v| v.state = GuardedState::Attention);
    let changed = fixture.read().unwrap().unwrap();
    assert_eq!(changed.sequence, first.sequence);
    assert_eq!(changed.observed_at, first.observed_at);
    assert_ne!(changed.payload_digest, first.payload_digest);
    assert_eq!(changed.state, GuardedState::Attention);
    // This library returns source facts; per-instance monotonic history belongs to
    // the owning supervisor and must reject this same-sequence equivocation there.
}

#[test]
fn one_writer_rejects_foreign_owner_and_content_replacement() {
    let fixture = Fixture::new();
    let mut reporter = fixture.reporter();
    reporter.publish(GuardedState::Ready).unwrap();
    assert!(
        Reporter::with_clock(
            fixture.binding.path().into(),
            fixture.binding.scope(),
            fixture.instance.clone(),
            fixture.pid,
            fixture.clock.clone()
        )
        .is_err()
    );
    let original = fs::read(fixture.binding.path()).unwrap();
    fixture.rewrite(|v| v.state = GuardedState::Attention);
    let replaced = fs::read(fixture.binding.path()).unwrap();
    assert!(reporter.publish(GuardedState::Ready).is_err());
    assert_eq!(fs::read(fixture.binding.path()).unwrap(), replaced);
    drop(reporter);
    fs::write(fixture.binding.path(), &original).unwrap();
    for (scope, instance, pid) in [
        (fixture.binding.scope(), id(99), fixture.pid),
        (
            fixture.binding.scope(),
            fixture.instance.clone(),
            fixture.pid + 1,
        ),
        (
            GuardedScope::Host {
                installation: id(1),
                host: name("host/b"),
                installation_identity: digest(2),
            },
            fixture.instance.clone(),
            fixture.pid,
        ),
    ] {
        assert!(
            Reporter::with_clock(
                fixture.binding.path().into(),
                scope,
                instance,
                pid,
                fixture.clock.clone()
            )
            .is_err()
        );
        assert_eq!(fs::read(fixture.binding.path()).unwrap(), original);
    }
    // Reacquiring this exact owner resumes its sequence; it does not reset to one.
    assert_eq!(
        fixture
            .reporter()
            .publish(GuardedState::Stopping)
            .unwrap()
            .sequence,
        Counter(2)
    );
}

#[test]
fn atomic_publication_keeps_old_readers_on_a_complete_old_file() {
    let fixture = Fixture::new();
    let mut reporter = fixture.reporter();
    reporter.publish(GuardedState::Starting).unwrap();
    let old_bytes = fs::read(fixture.binding.path()).unwrap();
    let mut old_file = fs::File::open(fixture.binding.path()).unwrap();
    let old_inode = old_file.metadata().unwrap().ino();
    let next = reporter.publish(GuardedState::Ready).unwrap();
    assert_ne!(
        old_inode,
        fs::metadata(fixture.binding.path()).unwrap().ino()
    );
    let mut still_old = Vec::new();
    old_file.read_to_end(&mut still_old).unwrap();
    assert_eq!(still_old, old_bytes);
    assert_eq!(fixture.read().unwrap(), Some(next));
    assert_eq!(
        fs::metadata(fixture.binding.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        fs::read_dir(fixture.binding.path().parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
    );
}

#[test]
fn malformed_or_removed_owned_status_cannot_be_silently_recreated() {
    let fixture = Fixture::new();
    let mut reporter = fixture.reporter();
    reporter.publish(GuardedState::Ready).unwrap();
    fs::remove_file(fixture.binding.path()).unwrap();
    assert!(
        reporter
            .publish(GuardedState::Stopped {
                reconciliation_required: false
            })
            .is_err()
    );
    assert!(!fixture.binding.path().exists());
    drop(reporter);
    fs::write(fixture.binding.path(), b"not a status envelope").unwrap();
    assert!(
        Reporter::with_clock(
            fixture.binding.path().into(),
            fixture.binding.scope(),
            fixture.instance.clone(),
            fixture.pid,
            fixture.clock.clone()
        )
        .is_err()
    );
    assert_eq!(
        fs::read(fixture.binding.path()).unwrap(),
        b"not a status envelope"
    );
}

#[test]
fn symlinks_special_files_and_missing_parent_are_errors_not_absence() {
    let fixture = Fixture::new();
    let parent = fixture.binding.path().parent().unwrap();
    let target = parent.join("unrelated");
    fs::write(&target, b"preserve me").unwrap();
    symlink(&target, fixture.binding.path()).unwrap();
    assert!(fixture.read().is_err());
    assert!(
        Reporter::with_clock(
            fixture.binding.path().into(),
            fixture.binding.scope(),
            fixture.instance.clone(),
            fixture.pid,
            fixture.clock.clone()
        )
        .is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"preserve me");
    fs::remove_file(&target).unwrap();
    assert!(fixture.read().is_err(), "dangling symlink is not absent");
    fs::remove_file(fixture.binding.path()).unwrap();
    fs::create_dir(fixture.binding.path()).unwrap();
    assert!(fixture.read().is_err());
    fs::remove_dir(fixture.binding.path()).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(fixture.binding.path())
            .status()
            .unwrap()
            .success()
    );
    assert!(fixture.read().is_err());
    fs::remove_file(fixture.binding.path()).unwrap();
    let missing_parent = fixture
        .binding
        .with_path(parent.join("missing/status.json"));
    assert!(
        Reader::with_clock(fixture.clock.clone())
            .read(&missing_parent, &fixture.instance, fixture.pid)
            .is_err()
    );
    let alias = parent.join("alias");
    symlink(parent, &alias).unwrap();
    let alias_binding = fixture.binding.with_path(alias.join("absent.json"));
    assert!(
        Reader::with_clock(fixture.clock.clone())
            .read(&alias_binding, &fixture.instance, fixture.pid)
            .is_err()
    );
    assert!(fixture.read().unwrap().is_none());
}

#[test]
fn reporter_clock_regression_and_sequence_overflow_do_not_replace_last_state() {
    let fixture = Fixture::new();
    let mut reporter = fixture.reporter();
    let first = reporter.publish(GuardedState::Ready).unwrap();
    let original = fs::read(fixture.binding.path()).unwrap();
    for bad in [
        time(first.observed_at.ticks_ns.0 - 1),
        TimePoint {
            clock_id: "test/other".into(),
            ticks_ns: first.observed_at.ticks_ns,
        },
    ] {
        fixture.clock.set(bad);
        assert!(reporter.publish(GuardedState::Attention).is_err());
        assert_eq!(fs::read(fixture.binding.path()).unwrap(), original);
    }
    fixture.clock.set(first.observed_at.clone());
    assert_eq!(
        reporter.publish(GuardedState::Stopping).unwrap().sequence,
        Counter(2)
    );
    drop(reporter);
    fixture.rewrite(|v| v.sequence = Counter(u64::MAX));
    let exhausted = fs::read(fixture.binding.path()).unwrap();
    assert!(
        fixture
            .reporter()
            .publish(GuardedState::Stopped {
                reconciliation_required: true
            })
            .is_err()
    );
    assert_eq!(fs::read(fixture.binding.path()).unwrap(), exhausted);
}

#[test]
fn status_path_opts_in_without_breaking_legacy_instance_id_only_environment() {
    use std::ffi::OsString;
    assert!(reporter::environment(None, None).unwrap().is_none());
    assert!(
        reporter::environment(None, Some(OsString::from("legacy-id")))
            .unwrap()
            .is_none()
    );
    for (path, instance) in [
        (
            Some(OsString::from("")),
            Some(OsString::from(id(3).to_string())),
        ),
        (Some(OsString::from("/tmp/status.json")), None),
        (
            Some(OsString::from("/tmp/status.json")),
            Some(OsString::from("invalid")),
        ),
    ] {
        assert!(reporter::environment(path, instance).is_err());
    }
    let parsed = reporter::environment(
        Some(OsString::from("/tmp/status.json")),
        Some(OsString::from(id(3).to_string())),
    )
    .unwrap()
    .unwrap();
    assert_eq!(parsed, (PathBuf::from("/tmp/status.json"), id(3)));
    let fixture = Fixture::new();
    assert!(
        Reporter::with_clock(
            PathBuf::from("relative-status.json"),
            fixture.binding.scope(),
            fixture.instance,
            fixture.pid,
            fixture.clock
        )
        .is_err()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn product_reader_and_reporter_clock_use_linux_boottime() {
    let fixture = Fixture::new();
    let clock = Arc::new(LinuxBoottime::new().unwrap());
    assert!(clock.now().unwrap().clock_id.starts_with("linux-boottime/"));
    let mut reporter = Reporter::with_clock(
        fixture.binding.path().into(),
        fixture.binding.scope(),
        fixture.instance.clone(),
        fixture.pid,
        clock,
    );
    reporter
        .as_mut()
        .unwrap()
        .publish(GuardedState::Starting)
        .unwrap();
    assert!(matches!(
        read(&fixture.binding, &fixture.instance, fixture.pid)
            .unwrap()
            .unwrap()
            .state,
        GuardedState::Starting
    ));
}
