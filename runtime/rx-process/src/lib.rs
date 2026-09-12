//! Process source validation and deterministic expansion. Execution authority remains in P.
pub mod bt_xml;
pub mod compile;
pub use compile::{Error, compile};
pub use model::*;
pub use rx_process_contract::frontier;
pub use rx_process_contract::model;
/// Compile only the process entry from the verifier's immutable content snapshot.
pub fn compile_package(
    package: &rx_package::VerifiedPackage,
    bindings: std::collections::BTreeMap<rx_domain::types::Name, ActionBinding>,
) -> Result<ResolvedProcess, Error> {
    let rx_package::EntryPoint::Process { source } = &package.manifest().entry else {
        return Err(Error {
            location: "package".into(),
            reason: "a process package is required".into(),
        });
    };
    let bytes = package.file(source).ok_or_else(|| Error {
        location: "package".into(),
        reason: "verified process entry missing".into(),
    })?;
    let source: ProcessSource = rx_domain::canonical::decode_json(bytes).map_err(|e| Error {
        location: "package/source".into(),
        reason: e.to_string(),
    })?;
    let mut process = compile(&source, bindings)?;
    for operation in process.bindings.keys() {
        if !package
            .manifest()
            .permissions
            .contains(&rx_package::Permission::OperationSubmit {
                operation: operation.clone(),
            })
        {
            return Err(Error {
                location: operation.to_string(),
                reason:
                    "operation submission is not declared in the verified package permission set"
                        .into(),
            });
        }
    }
    process.package_digest = Some(package.digest());
    Ok(process)
}
