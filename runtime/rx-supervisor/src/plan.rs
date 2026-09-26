use crate::{Error, Result, model::*};
use rx_domain::{canonical, types::*};
use rx_solution_catalog::DeviceCatalog;
use std::collections::{BTreeMap, BTreeSet};
impl Plan {
    pub fn validate(
        &self,
        programs: &BTreeMap<Name, Program>,
        support: &DeviceCatalog,
    ) -> Result<Digest> {
        support
            .validate()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        if self.schema.as_str() != "rx.solutions-process-plan.v1"
            || self.processes.is_empty()
            || self.processes.len() > 32
            || self.profiles.len() > 22
            || self.profiles.iter().collect::<BTreeSet<_>>().len() != self.profiles.len()
        {
            return Err(Error::Invalid("plan shape".into()));
        }
        if self.profiles.iter().any(|p| support.profile(p).is_none()) {
            return Err(Error::Invalid("unknown required support profile".into()));
        }
        let ids: BTreeSet<_> = self.processes.iter().map(|p| &p.id).collect();
        if ids.len() != self.processes.len() {
            return Err(Error::Invalid("duplicate process".into()));
        }
        for p in &self.processes {
            let program = programs
                .get(&p.program)
                .ok_or_else(|| Error::Invalid("program not in release catalog".into()))?;
            if program.id.as_str() == "rx/dhi-pty-simulation"
                && (self.environment != Environment::Simulation || p.restart_limit.0 != 0)
            {
                return Err(Error::Invalid(
                    "DHI_SIMULATION_ONLY_EXPLICIT_SESSION; physical plans and automatic restart are unsupported".into(),
                ));
            }
            if program.id.as_str() == "rx/ai-worker-l3-simulation"
                && (self.environment != Environment::Simulation || p.restart_limit.0 != 0)
            {
                return Err(Error::Invalid(
                    "AI_WORKER_L3_SIMULATION_ONLY_EXPLICIT_SESSION; physical plans and RX automatic restart are unsupported".into(),
                ));
            }
            if program.id.as_str() == "rx/open-manipulator-l3-simulation"
                && (self.environment != Environment::Simulation || p.restart_limit.0 != 0)
            {
                return Err(Error::Invalid("OPEN_MANIPULATOR_L3_SIMULATION_ONLY; physical plans and RX automatic restart are unsupported".into()));
            }
            if let Some(policy) = &program.decision_policy {
                policy.fingerprint().map_err(Error::Invalid)?;
            }
            if let Some(requirements) = &program.execution_requirements {
                requirements.validate()?;
            }
            if let Some(contract) = &program.functional_readiness {
                contract.validate().map_err(Error::Invalid)?;
            }
            if p.depends_on.iter().any(|d| !ids.contains(d) || d == &p.id)
                || p.depends_on.iter().collect::<BTreeSet<_>>().len() != p.depends_on.len()
                || !(100..=30_000).contains(&p.startup_timeout_ms.0)
                || !(100..=30_000).contains(&p.shutdown_timeout_ms.0)
                || p.restart_limit.0 > 3
                || !(100..=30_000).contains(&p.restart_backoff_ms.0)
                || (program.effect != Effect::NonActuating && p.restart_limit.0 != 0)
            {
                return Err(Error::Invalid("dependency/timing/restart policy".into()));
            }
            p.launch(
                program,
                Id::new(uuid::Uuid::new_v4().to_string())
                    .map_err(|e| Error::Invalid(e.to_string()))?,
            )?;
        }
        let mut resolved = BTreeSet::new();
        loop {
            let before = resolved.len();
            for p in &self.processes {
                if p.depends_on.iter().all(|d| resolved.contains(d)) {
                    resolved.insert(p.id.clone());
                }
            }
            if resolved.len() == ids.len() {
                break;
            }
            if resolved.len() == before {
                return Err(Error::Invalid("process dependency cycle".into()));
            }
        }
        canonical::digest(
            "RX-SUPERVISOR-PLAN-v1",
            &(
                self,
                self.processes
                    .iter()
                    .map(|p| (&p.program, &programs[&p.program]))
                    .collect::<BTreeMap<_, _>>(),
                self.profiles
                    .iter()
                    .map(|id| support.profile(id).expect("validated profile"))
                    .collect::<Vec<_>>(),
            ),
        )
        .map_err(|e| Error::Invalid(e.to_string()))
    }
}
impl Process {
    pub fn launch(&self, p: &Program, instance: Id) -> Result<Launch> {
        if p.id != self.program
            || self.parameters.keys().collect::<BTreeSet<_>>() != p.arguments.keys().collect()
        {
            return Err(Error::Invalid("program parameters differ".into()));
        }
        let mut arguments = p.fixed_arguments.clone();
        let mut port = None;
        for (name, spec) in &p.arguments {
            let value = &self.parameters[name];
            if value.is_empty() || value.len() > 256 || value.contains('\0') {
                return Err(Error::Invalid("parameter value".into()));
            }
            match spec {
                Argument::Choice { flag, values } => {
                    if !values.contains(value) {
                        return Err(Error::Invalid("parameter choice".into()));
                    }
                    arguments.extend([flag.clone(), value.clone()]);
                }
                Argument::Port { flag } => {
                    let number = value
                        .parse::<u16>()
                        .map_err(|_| Error::Invalid("port".into()))?;
                    if number < 1024 || number.to_string() != *value {
                        return Err(Error::Invalid("port".into()));
                    }
                    port = Some(number);
                    arguments.extend([flag.clone(), value.clone()]);
                }
            }
        }
        if let ReadyProbe::HttpStatus { port_parameter } = &p.ready
            && !matches!(p.arguments.get(port_parameter), Some(Argument::Port { .. }))
        {
            return Err(Error::Invalid("readiness port binding".into()));
        }
        if (p.effect == Effect::ProtocolGuardedService)
            != matches!(p.ready, ReadyProbe::GuardedStatus(_))
        {
            return Err(Error::Invalid(
                "protocol-guarded effect requires its typed status binding".into(),
            ));
        }
        let ready = if let ReadyProbe::GuardedStatus(binding) = &p.ready {
            let path = binding.path();
            if !path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(Error::Invalid(
                    "absolute release-owned status path required".into(),
                ));
            }
            let parent = path
                .parent()
                .ok_or_else(|| Error::Invalid("status parent required".into()))?;
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| Error::Invalid("status filename required".into()))?;
            ReadyProbe::GuardedStatus(
                binding.with_path(parent.join(format!("{stem}.{instance}.json"))),
            )
        } else {
            p.ready.clone()
        };
        Ok(Launch {
            selection: self.id.clone(),
            instance,
            effect: p.effect,
            executable: p.executable.clone(),
            executable_sha256: p.executable_sha256,
            files: p.files.clone(),
            arguments,
            ready,
            port,
        })
    }
}
