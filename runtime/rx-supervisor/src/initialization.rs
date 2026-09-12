//! Named release-generated metadata-only initializers. Entered never means safe to retry.
use crate::{
    Error, Result,
    builtin::{Initializer, ServiceRole},
    process::verify,
};
use rx_domain::{canonical, types::*};
use rx_ports::{Document, Repository, StoreError};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

const KEY: &str = "service-initialization/state";
const SCHEMA: &str = "rx.solutions-service-initialization.v1";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Pending,
    Entered,
    Completed,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub program: Name,
    pub role: ServiceRole,
    pub instance: Id,
    pub phase: Phase,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub digest: Digest,
    pub steps: Vec<Step>,
}
fn name(s: &str) -> Name {
    Name::new(s).expect("fixed name")
}
fn identifier() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID")
}
fn decode(row: &rx_ports::Record) -> rx_ports::Result<State> {
    if row.document.schema != name(SCHEMA) {
        return Err(StoreError::Integrity(
            "initialization schema differs".into(),
        ));
    }
    let state: State = canonical::decode_json(
        &canonical::bytes(&row.document.value).map_err(|e| StoreError::Integrity(e.to_string()))?,
    )
    .map_err(|e| StoreError::Integrity(e.to_string()))?;
    if state.steps.is_empty()
        || state.steps.len() > 32
        || state
            .steps
            .iter()
            .map(|s| &s.program)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != state.steps.len()
        || state
            .steps
            .iter()
            .map(|s| &s.instance)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != state.steps.len()
        || state.steps.iter().any(|step| match step.phase {
            Phase::Pending | Phase::Entered => step.exit_code.is_some() || step.error.is_some(),
            Phase::Completed => step.exit_code != Some(0) || step.error.is_some(),
            Phase::Failed => step.error.is_none(),
        })
    {
        return Err(StoreError::Integrity(
            "initializer identity/phase differs".into(),
        ));
    }
    Ok(state)
}
fn document(value: &State) -> rx_ports::Result<Document> {
    Ok(Document {
        schema: name(SCHEMA),
        value: serde_json::to_value(value).map_err(|e| StoreError::Invalid(e.to_string()))?,
    })
}
pub fn digest(plan: Digest, commands: &[Initializer]) -> Result<Digest> {
    canonical::digest("RX-SERVICE-INITIALIZATION-v1", &(plan, commands))
        .map_err(|e| Error::Invalid(e.to_string()))
}
fn inspect<R: Repository>(
    repository: &mut R,
    expected: Digest,
    commands: &[Initializer],
) -> Result<State> {
    Ok(repository.transact(|tx| {
        let row = tx
            .get(&name(KEY))?
            .ok_or_else(|| StoreError::Integrity("required initialization state absent".into()))?;
        let state = decode(&row)?;
        if state.digest != expected {
            return Err(StoreError::KeyConflict);
        }
        check_commands(&state, commands)?;
        Ok(state)
    })?)
}
pub fn require_complete<R: Repository>(
    repository: &mut R,
    expected: Digest,
    commands: &[Initializer],
) -> Result<State> {
    let state = inspect(repository, expected, commands)?;
    if state
        .steps
        .iter()
        .any(|s| s.phase != Phase::Completed || s.exit_code != Some(0))
    {
        return Err(Error::Reconciliation(
            "fixed service initialization is incomplete; no automatic init replay".into(),
        ));
    }
    Ok(state)
}
fn check_commands(state: &State, commands: &[Initializer]) -> rx_ports::Result<()> {
    if state.steps.len() != commands.len()
        || state
            .steps
            .iter()
            .zip(commands)
            .any(|(step, command)| step.program != command.id || step.role != command.role)
    {
        return Err(StoreError::Integrity(
            "initializer list or order differs".into(),
        ));
    }
    Ok(())
}
fn change<R: Repository>(
    repository: &mut R,
    expected: Digest,
    commands: &[Initializer],
    index: usize,
    phase: Phase,
    code: Option<i32>,
    error: Option<String>,
) -> Result<()> {
    repository.transact(|tx| {
        let row = tx
            .get(&name(KEY))?
            .ok_or_else(|| StoreError::Integrity("initialization state missing".into()))?;
        let mut state = decode(&row)?;
        check_commands(&state, commands)?;
        if state.digest != expected || index >= state.steps.len() {
            return Err(StoreError::KeyConflict);
        }
        let step = &mut state.steps[index];
        if !matches!(
            (step.phase, phase),
            (Phase::Pending, Phase::Entered) | (Phase::Entered, Phase::Completed | Phase::Failed)
        ) {
            return Err(StoreError::Invalid(
                "initialization transition differs".into(),
            ));
        }
        step.phase = phase;
        step.exit_code = code;
        step.error = error;
        tx.put(&name(KEY), Some(row.revision), &document(&state)?)?;
        tx.append(&identifier(), &document(&state)?)?;
        Ok(())
    })?;
    Ok(())
}

pub async fn run<R: Repository>(
    repository: &mut R,
    expected: Digest,
    commands: &[Initializer],
    logs: &Path,
) -> Result<State> {
    if commands.is_empty()
        || commands.len() > 32
        || commands
            .iter()
            .any(|c| !c.id.as_str().starts_with("rx/service/"))
        || commands
            .iter()
            .map(|c| &c.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != commands.len()
    {
        return Err(Error::Invalid(
            "bounded release-generated service initializers required".into(),
        ));
    }
    let (head, records) = repository.snapshot()?;
    if records.is_empty() && head.0 == 0 {
        repository.transact(|tx| {
            let state = State {
                digest: expected,
                steps: commands
                    .iter()
                    .map(|c| Step {
                        program: c.id.clone(),
                        role: c.role,
                        instance: identifier(),
                        phase: Phase::Pending,
                        exit_code: None,
                        error: None,
                    })
                    .collect(),
            };
            tx.put(&name(KEY), None, &document(&state)?)?;
            Ok(())
        })?;
    }
    std::fs::create_dir_all(logs)?;
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    for (index, command) in commands.iter().enumerate() {
        let state = inspect(repository, expected, commands)?;
        let step = state
            .steps
            .get(index)
            .filter(|s| s.program == command.id)
            .ok_or_else(|| Error::Reconciliation("initializer identity changed".into()))?;
        if step.phase == Phase::Completed {
            continue;
        }
        if step.phase != Phase::Pending {
            return Err(Error::Reconciliation("initializer entered or failed; existing metadata requires inspection, never replay".into()));
        }
        verify(&command.executable, command.executable_sha256)?;
        for (path, hash) in &command.files {
            verify(path, *hash)?;
        }
        #[cfg(unix)]
        tokio::select! {
            biased;
            _=term.recv()=>return Err(Error::Reconciliation("initialization interrupted before the next child was started".into())),
            _=tokio::signal::ctrl_c()=>return Err(Error::Reconciliation("initialization interrupted before the next child was started".into())),
            _=std::future::ready(())=>{},
        }
        change(
            repository,
            expected,
            commands,
            index,
            Phase::Entered,
            None,
            None,
        )?;
        let stdout = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(logs.join(format!("{}.stdout.log", step.instance)))?;
        let stderr = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(logs.join(format!("{}.stderr.log", step.instance)))?;
        let mut child = match Command::new(&command.executable)
            .args(&command.arguments)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                change(
                    repository,
                    expected,
                    commands,
                    index,
                    Phase::Failed,
                    None,
                    Some(error.to_string()),
                )?;
                return Err(error.into());
            }
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut interrupted = false;
        let mut termination_sent = false;
        let mut last_signal_error = None;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            #[cfg(unix)]
            tokio::select! {
                _=tokio::time::sleep(Duration::from_millis(50))=>{},
                _=term.recv()=>{interrupted=true;},
                _=tokio::signal::ctrl_c()=>{interrupted=true;},
            }
            #[cfg(not(unix))]
            tokio::time::sleep(Duration::from_millis(50)).await;
            if !termination_sent && (interrupted || Instant::now() >= deadline) {
                interrupted = true;
                // Metadata initialization has no native control. Still retain the owned handle;
                // TERM requests interruption, and timeout is never a successful completion.
                if let Err(error) =
                    attempt_termination(&mut child, &mut termination_sent, signal_owned_pid)
                {
                    let text = error.to_string();
                    if last_signal_error.as_ref() != Some(&text) {
                        eprintln!(
                            "initializer TERM delivery failed; child retained for retry: {text}"
                        );
                        last_signal_error = Some(text);
                    }
                }
            }
        };
        let completed = status.success() && !interrupted;
        change(
            repository,
            expected,
            commands,
            index,
            if completed {
                Phase::Completed
            } else {
                Phase::Failed
            },
            status.code(),
            (!completed)
                .then(|| "initializer failed or interrupted; partial installation retained".into()),
        )?;
        if !completed {
            return Err(Error::Reconciliation(
                "service initialization incomplete".into(),
            ));
        }
    }
    require_complete(repository, expected, commands)
}

/// Only a confirmed delivery (or an already observed exit) suppresses the next retry.
fn attempt_termination(
    child: &mut Child,
    sent: &mut bool,
    deliver: impl FnOnce(u32) -> Result<()>,
) -> Result<()> {
    if *sent {
        return Ok(());
    }
    if child.try_wait()?.is_some() {
        *sent = true;
        return Ok(());
    }
    deliver(child.id())?;
    *sent = true;
    Ok(())
}
#[cfg(unix)]
fn signal_owned_pid(pid: u32) -> Result<()> {
    let status = Command::new("/bin/kill")
        .env_clear()
        .args(["-TERM", "--", &pid.to_string()])
        .status()?;
    check_signal_status(status)
}
#[cfg(not(unix))]
fn signal_owned_pid(_pid: u32) -> Result<()> {
    Err(Error::Invalid(
        "initializer termination requires Unix signals".into(),
    ))
}
#[cfg(unix)]
fn check_signal_status(status: ExitStatus) -> Result<()> {
    if !status.success() {
        return Err(Error::Reconciliation(format!(
            "owned initializer TERM was not confirmed: {status}"
        )));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod termination_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    struct OwnedSleeper(Child);
    impl Drop for OwnedSleeper {
        fn drop(&mut self) {
            if matches!(self.0.try_wait(), Ok(None)) {
                let _ = signal_owned_pid(self.0.id());
            }
            let _ = self.0.wait(); // The fixture also exits naturally within five seconds.
        }
    }

    #[test]
    fn failed_term_keeps_the_same_owned_child_retryable_until_delivery_is_confirmed() {
        let mut child = OwnedSleeper(
            Command::new("/bin/sleep")
                .arg("5")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let pid = child.0.id();
        let mut sent = false;
        let mut attempts = Vec::new();
        assert!(
            attempt_termination(&mut child.0, &mut sent, |actual| {
                attempts.push(actual);
                check_signal_status(ExitStatus::from_raw(1 << 8))
            })
            .is_err()
        );
        assert!(!sent);
        assert_eq!(child.0.id(), pid);
        assert!(child.0.try_wait().unwrap().is_none());

        attempt_termination(&mut child.0, &mut sent, |actual| {
            attempts.push(actual);
            signal_owned_pid(actual)
        })
        .unwrap();
        assert!(sent);
        assert_eq!(attempts, vec![pid, pid]);
        attempt_termination(&mut child.0, &mut sent, |_| {
            panic!("confirmed TERM must not be repeated")
        })
        .unwrap();
    }

    #[test]
    fn failed_signal_command_status_is_not_a_successful_delivery() {
        assert!(check_signal_status(ExitStatus::from_raw(1 << 8)).is_err());
        assert!(check_signal_status(ExitStatus::from_raw(0)).is_ok());
    }
}
