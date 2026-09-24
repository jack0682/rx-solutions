//! Birth observations are captured only from an owned, unreaped Child. Stored
//! identities are inert history, not process ownership or cryptographic truth.
use crate::execution::Request;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredProcessIdentity {
    pub(crate) scope: Scope,
    pub(crate) pid: u32,
    pub(crate) start_ticks: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    boot: String,
    pid_namespace: String,
    time_namespace: String,
    user_namespace: String,
    mount_namespace: String,
    uid: u32,
    euid: u32,
    proc_mount: String,
    init_start_ticks: u64,
    time_offsets: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct OwnedProcessIdentity {
    request: Request,
    stored: StoredProcessIdentity,
}
impl OwnedProcessIdentity {
    pub(crate) fn matches(&self, request: &Request, pid: u32) -> bool {
        self.request == *request && self.stored.pid == pid
    }
    pub(crate) fn stored(self) -> StoredProcessIdentity {
        self.stored
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "SCREAMING_SNAKE_CASE",
    deny_unknown_fields
)]
pub enum InvestigationOutcome {
    MatchingProcessPresent,
    OriginalNotRunning { reason: String },
    Unverifiable { reason: String },
}
impl InvestigationOutcome {
    pub fn permits_closure(&self) -> bool {
        matches!(self, Self::OriginalNotRunning { .. })
    }
    pub(crate) fn unknown(reason: impl Into<String>) -> Self {
        Self::Unverifiable {
            reason: reason.into(),
        }
    }
}
pub(crate) fn capture(
    child: &mut std::process::Child,
    request: &Request,
) -> Option<Box<OwnedProcessIdentity>> {
    #[cfg(target_os = "linux")]
    {
        if child.try_wait().ok()?.is_some() {
            return None;
        }
        let scope = linux::scope().ok()?;
        let first = linux::stat(child.id()).ok()?;
        let second = linux::stat(child.id()).ok()?;
        if first.start != second.start
            || second.zombie()
            || first.zombie()
            || child.try_wait().ok()?.is_some()
            || scope != linux::scope().ok()?
        {
            return None;
        }
        Some(Box::new(OwnedProcessIdentity {
            request: request.clone(),
            stored: StoredProcessIdentity {
                scope,
                pid: child.id(),
                start_ticks: first.start,
            },
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (child, request);
        None
    }
}
pub(crate) fn investigate(saved: Option<&StoredProcessIdentity>) -> InvestigationOutcome {
    let Some(saved) = saved else {
        return InvestigationOutcome::unknown("birth-identity-missing-permanent-no-backfill");
    };
    #[cfg(target_os = "linux")]
    {
        linux::investigate(saved).unwrap_or_else(InvestigationOutcome::unknown)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = saved;
        InvestigationOutcome::unknown("linux-investigation-unsupported")
    }
}
pub(crate) fn scope_matches(saved: &StoredProcessIdentity) -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::scope().is_ok_and(|scope| scope == saved.scope)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = saved;
        false
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::fs;
    type Check<T> = std::result::Result<T, String>;
    fn read(path: impl AsRef<std::path::Path>) -> Check<String> {
        fs::read_to_string(path).map_err(|e| format!("proc-read-unverifiable: {e}"))
    }
    fn namespace(kind: &str) -> Check<String> {
        fs::read_link(format!("/proc/self/ns/{kind}"))
            .map_err(|e| format!("namespace-unverifiable: {e}"))?
            .into_os_string()
            .into_string()
            .map_err(|_| "namespace-encoding".into())
    }
    pub(super) fn scope() -> Check<Scope> {
        coherent_scope(read_scope()?, read_scope()?)
    }
    fn coherent_scope(first: Scope, second: Scope) -> Check<Scope> {
        if first != second {
            return Err("scoped-context-read-race".into());
        }
        Ok(first)
    }
    fn read_scope() -> Check<Scope> {
        let self_pid = fs::read_link("/proc/self").map_err(|e| e.to_string())?;
        if self_pid.to_str() != Some(std::process::id().to_string().as_str()) {
            return Err("proc-pid-view-mismatch".into());
        }
        let mounts = read("/proc/self/mountinfo")?;
        let mut root = None;
        for line in mounts.lines() {
            let (left, right) = line.split_once(" - ").ok_or("proc-mount-format")?;
            let fields: Vec<_> = left.split_whitespace().collect();
            if fields.len() < 6 {
                return Err("proc-mount-format".into());
            }
            let path = fields[4];
            if path == "/proc" {
                if root.is_some()
                    || fields[3] != "/"
                    || !right.starts_with("proc proc ")
                    || line.contains("hidepid=")
                    || line.contains("subset=")
                {
                    return Err("proc-visibility-unverifiable".into());
                }
                root = Some(line.to_owned());
            } else if path.strip_prefix("/proc/").is_some_and(|p| {
                let first = p.split('/').next().unwrap_or("");
                first == "self" || first == "thread-self" || first.parse::<u32>().is_ok()
            }) {
                return Err("proc-pid-overlay-unverifiable".into());
            }
        }
        let boot = read("/proc/sys/kernel/random/boot_id")?.trim().to_owned();
        uuid::Uuid::parse_str(&boot).map_err(|_| "boot-id-unverifiable")?;
        let init = stat(1)?;
        if init.zombie() {
            return Err("namespace-init-unverifiable".into());
        }
        let time_offsets = read("/proc/self/timens_offsets")?;
        if time_offsets.len() > 256
            || !time_offsets.contains("monotonic")
            || !time_offsets.contains("boottime")
        {
            return Err("time-offsets-unverifiable".into());
        }
        Ok(Scope {
            init_start_ticks: init.start,
            time_offsets,
            boot,
            pid_namespace: namespace("pid")?,
            time_namespace: namespace("time")?,
            user_namespace: namespace("user")?,
            mount_namespace: namespace("mnt")?,
            uid: rustix::process::getuid().as_raw(),
            euid: rustix::process::geteuid().as_raw(),
            proc_mount: root.ok_or("proc-mount-missing")?,
        })
    }
    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Stat {
        pub(super) start: u64,
        state: char,
    }
    impl Stat {
        pub(super) fn zombie(&self) -> bool {
            matches!(self.state, 'Z' | 'X' | 'x')
        }
    }
    fn parse_stat(text: &str, pid: u32) -> Check<Stat> {
        let (head, tail) = text.rsplit_once(") ").ok_or("proc-stat-format")?;
        if !head.starts_with(&format!("{pid} (")) {
            return Err("proc-stat-pid-mismatch".into());
        }
        let fields: Vec<_> = tail.split_whitespace().collect();
        let start = fields
            .get(19)
            .ok_or("proc-stat-start-missing")?
            .parse()
            .map_err(|_| "proc-stat-start-format")?;
        let state = fields
            .first()
            .filter(|s| s.len() == 1)
            .and_then(|s| s.chars().next())
            .ok_or("proc-stat-state-format")?;
        Ok(Stat { start, state })
    }
    pub(super) fn stat(pid: u32) -> Check<Stat> {
        parse_stat(&read(format!("/proc/{pid}/stat"))?, pid)
    }
    pub(super) fn investigate(saved: &StoredProcessIdentity) -> Check<InvestigationOutcome> {
        let before = scope()?;
        if before != saved.scope {
            return Err("saved-namespace-not-observable-in-current-scope".into());
        }
        // pidfd_open's ESRCH distinguishes kernel absence from hidden proc files.
        // This observation handle is never installed into the execution backend,
        // signalled, or treated as an owned Child.
        let pid = rustix::process::Pid::from_raw(saved.pid as i32).ok_or("saved-pid-invalid")?;
        let outcome = match rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()) {
            Err(rustix::io::Errno::SRCH) => InvestigationOutcome::OriginalNotRunning {
                reason: "scoped-kernel-pid-absent".into(),
            },
            Err(e) => return Err(format!("pidfd-observation-unverifiable: {e}")),
            Ok(_observation) => {
                let first = stat(saved.pid).map_err(|e| format!("process-read-race: {e}"))?;
                let second = stat(saved.pid).map_err(|e| format!("process-read-race: {e}"))?;
                classify(saved.start_ticks, &first, &second)
            }
        };
        if scope()? != before {
            return Err("scoped-context-read-race".into());
        }
        Ok(outcome)
    }
    fn classify(saved: u64, first: &Stat, second: &Stat) -> InvestigationOutcome {
        if first.zombie() || second.zombie() {
            return InvestigationOutcome::unknown("process-zombie-unverifiable");
        }
        // State may legitimately change R/S; identity change while reading may not.
        if first.start != second.start {
            return InvestigationOutcome::unknown("process-read-race");
        }
        if first.start != saved {
            InvestigationOutcome::OriginalNotRunning {
                reason: "scoped-pid-reused-different-starttime".into(),
            }
        } else {
            InvestigationOutcome::MatchingProcessPresent
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn zombie_and_read_race_are_distinct_unverifiable_findings() {
            let live = Stat {
                start: 10,
                state: 'S',
            };
            assert_eq!(
                classify(
                    10,
                    &live,
                    &Stat {
                        start: 10,
                        state: 'Z'
                    }
                ),
                InvestigationOutcome::unknown("process-zombie-unverifiable")
            );
            assert_eq!(
                classify(
                    10,
                    &live,
                    &Stat {
                        start: 11,
                        state: 'S'
                    }
                ),
                InvestigationOutcome::unknown("process-read-race")
            );
        }
        #[test]
        fn foreign_namespace_neither_matches_nor_proves_absence() {
            let mut scope = scope().unwrap();
            scope.pid_namespace.push_str("-foreign");
            for pid in [std::process::id(), i32::MAX as u32] {
                let saved = StoredProcessIdentity {
                    scope: scope.clone(),
                    pid,
                    start_ticks: 1,
                };
                assert_eq!(
                    investigate(&saved),
                    Err("saved-namespace-not-observable-in-current-scope".into())
                );
            }
        }
        #[test]
        fn every_saved_scope_dimension_is_required_before_absence() {
            let saved = StoredProcessIdentity {
                scope: scope().unwrap(),
                pid: i32::MAX as u32,
                start_ticks: 1,
            };
            for field in [
                "boot",
                "time_namespace",
                "user_namespace",
                "mount_namespace",
                "uid",
                "euid",
                "proc_mount",
                "time_offsets",
                "init_start_ticks",
            ] {
                let mut json = serde_json::to_value(&saved).unwrap();
                if field == "uid" || field == "euid" || field == "init_start_ticks" {
                    json["scope"][field] = serde_json::json!(u32::MAX);
                } else {
                    json["scope"][field] = serde_json::json!("different-context");
                }
                let altered: StoredProcessIdentity = serde_json::from_value(json).unwrap();
                assert_eq!(
                    investigate(&altered),
                    Err("saved-namespace-not-observable-in-current-scope".into()),
                    "{field}"
                );
                let mut missing = serde_json::to_value(&saved).unwrap();
                missing["scope"].as_object_mut().unwrap().remove(field);
                assert!(serde_json::from_value::<StoredProcessIdentity>(missing).is_err());
            }
        }
        #[test]
        fn namespace_lifetime_read_race_has_its_own_rejection() {
            let first = scope().unwrap();
            let mut second = first.clone();
            second.init_start_ticks += 1;
            assert_eq!(
                coherent_scope(first, second),
                Err("scoped-context-read-race".into())
            );
        }
        #[test]
        fn stat_comm_parentheses_do_not_shift_starttime() {
            let text = format!("123 (odd ) (name) S {} 42 0", vec!["0"; 18].join(" "));
            assert_eq!(parse_stat(&text, 123).unwrap().start, 42);
        }
    }
}
