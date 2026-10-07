//! Common signed template package facts, independent of the selected native provider.
use crate::{HostError, Result};
use rx_domain::{canonical, intent::Body, types::*};
use rx_package::{EntryPoint, PackagePath, VerifiedPackage};
use rx_process_contract::execution_v2::{self as v2, TemplateCatalog};
pub struct Templates {
    pub(crate) manifest: Digest,
    pub(crate) signature: Digest,
    pub(crate) reference: ArtifactRef,
    pub(crate) catalog: TemplateCatalog,
}
fn invalid(e: impl std::fmt::Display) -> HostError {
    HostError::Invalid(e.to_string())
}
impl Templates {
    pub fn verify(package: &VerifiedPackage) -> Result<Self> {
        let manifest = package.manifest();
        let EntryPoint::DeviceReference {
            family,
            profiles,
            adapter,
        } = &manifest.entry
        else {
            return Err(HostError::Guard);
        };
        if manifest.schema.as_str() != "rx.package.v2"
            || profiles.len() != 1
            || !manifest.dependencies.is_empty()
        {
            return Err(HostError::Guard);
        }
        let path = PackagePath::new("execution-template-catalog.json").map_err(invalid)?;
        let bytes = package.file(&path).ok_or(HostError::Guard)?;
        let catalog = TemplateCatalog::decode(bytes).map_err(invalid)?;
        if catalog.documents[&Name::new("family").expect("role")].path != family.as_str()
            || catalog.documents[&Name::new("profile").expect("role")].path != profiles[0].as_str()
            || catalog.documents[&Name::new("adapter").expect("role")].path != adapter.as_str()
        {
            return Err(HostError::Guard);
        }
        let reference = ArtifactRef {
            schema_id: Name::new(v2::TEMPLATE_CATALOG_SCHEMA).expect("schema"),
            sha256: rx_package::content_digest(bytes),
            size_bytes: Counter(bytes.len() as u64),
        };
        let declared = |r: &ArtifactRef| -> Result<()> {
            if !manifest.assets.contains(r)
                || !package.files().any(|(path, b)| {
                    b.len() as u64 == r.size_bytes.0
                        && rx_package::content_digest(b) == r.sha256
                        && manifest
                            .files
                            .iter()
                            .any(|f| &f.path == path && !f.executable)
                })
            {
                return Err(HostError::Guard);
            }
            Ok(())
        };
        declared(&reference)?;
        for document in catalog.documents.values() {
            declared(&document.artifact)?;
            let bytes = package
                .file(&PackagePath::new(&document.path).map_err(invalid)?)
                .ok_or(HostError::Guard)?;
            v2::verify_artifact(bytes, &document.artifact, 2 * 1024 * 1024).map_err(invalid)?;
        }
        for template in catalog.templates.values() {
            let Body::Program(p) = &template.action.intent.body else {
                return Err(HostError::Guard);
            };
            declared(&p.program)?;
            declared(&p.parameter_set)?;
        }
        Ok(Self {
            manifest: package.digest(),
            signature: rx_package::content_digest(
                &canonical::bytes(package.signature()).map_err(invalid)?,
            ),
            reference,
            catalog,
        })
    }
    pub fn catalog(&self) -> &TemplateCatalog {
        &self.catalog
    }
}
