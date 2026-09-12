use crate::{Binding, Environment};
use rx_domain::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedFile {
    pub path: PathBuf,
    pub sha256: Digest,
}
impl PinnedFile {
    pub fn read(&self, secret: bool) -> Result<Vec<u8>> {
        if !self.path.is_absolute() {
            return Err("absolute pinned file path required".into());
        }
        let meta = std::fs::symlink_metadata(&self.path)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1_048_576 {
            return Err("pinned file type/size".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if secret && meta.permissions().mode() & 0o077 != 0 {
                return Err("private key must have owner-only access".into());
            }
        }
        let file = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("file name")?;
        let data = rx_package::directory::read_relative_file(
            self.path.parent().ok_or("parent")?,
            &rx_package::PackagePath::new(file)?,
            1_048_576,
        )?;
        if rx_package::content_digest(&data) != self.sha256 {
            return Err("pinned file digest mismatch".into());
        }
        Ok(data)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Backend {
    FileSimulation,
    JtcPackage {
        directory: PathBuf,
        manifest_digest: Digest,
        policy: PinnedFile,
    },
    MelsecPackage {
        directory: PathBuf,
        manifest_digest: Digest,
        policy: PinnedFile,
    },
    ValidatedDriver {
        profile: Name,
        driver_digest: Digest,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsFiles {
    pub certificate: PinnedFile,
    pub key: PinnedFile,
    pub ca: PinnedFile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    pub uri: String,
    pub server_name: String,
    pub tls: TlsFiles,
    pub store_generation: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema: Name,
    pub installation: Id,
    pub release_digest: Digest,
    pub host: Name,
    pub bind: SocketAddr,
    pub data_directory: PathBuf,
    pub runtime_directory: PathBuf,
    pub bindings: PinnedFile,
    pub backend: Backend,
    pub tls: TlsFiles,
    pub allowed_platform_certificates: BTreeMap<Digest, Name>,
    pub publisher: Option<Publication>,
    pub publication_drain_ms: Counter,
}
pub struct Loaded {
    pub config: Configuration,
    pub bindings: Vec<Binding>,
    pub tls: crate::rpc::TlsMaterial,
    pub publisher: Option<crate::publication::Configuration>,
    pub identity: Digest,
}
impl Loaded {
    pub fn read(path: &Path) -> Result<Self> {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1_048_576 {
            return Err("startup file type/size".into());
        }
        let config: Configuration = canonical::decode_json(&std::fs::read(path)?)?;
        if config.schema.as_str() != "rx.host-startup.v1"
            || !config.data_directory.is_absolute()
            || !config.runtime_directory.is_absolute()
            || config.data_directory.starts_with(&config.runtime_directory)
            || config.runtime_directory.starts_with(&config.data_directory)
            || config.allowed_platform_certificates.is_empty()
            || config.allowed_platform_certificates.len() > 128
            || config.publication_drain_ms.0 > 30000
        {
            return Err("invalid Host startup configuration".into());
        }
        let bindings: Vec<Binding> = canonical::decode_json(&config.bindings.read(false)?)?;
        if bindings.is_empty()
            || bindings.len() > 64
            || bindings
                .iter()
                .any(|b| b.host != config.host || b.platform != bindings[0].platform)
            || config
                .allowed_platform_certificates
                .values()
                .any(|p| p != &bindings[0].platform)
        {
            return Err("Host bindings/peer scope differs".into());
        }
        if matches!(config.backend, Backend::FileSimulation)
            && bindings
                .iter()
                .any(|b| b.environment != Environment::Simulation)
        {
            return Err("file simulation cannot serve a physical binding".into());
        }
        let tls = crate::rpc::TlsMaterial {
            server_certificate_pem: config.tls.certificate.read(false)?,
            server_key_pem: config.tls.key.read(true)?,
            client_ca_pem: config.tls.ca.read(false)?,
        };
        // Validate certificate/key parsing before creating authority storage.
        crate::rpc::validate_tls(&tls)?;
        let publisher = if let Some(p) = &config.publisher {
            if !p.uri.starts_with("https://") || p.server_name.is_empty() {
                return Err("publication requires HTTPS and server name".into());
            }
            Some(crate::publication::Configuration {
                destination: crate::publication::Destination {
                    platform: bindings[0].platform.clone(),
                    installation: config.installation.clone(),
                    store_generation: p.store_generation.clone(),
                },
                release_digest: config.release_digest,
                endpoint: crate::publication::Endpoint {
                    uri: p.uri.clone(),
                    server_name: p.server_name.clone(),
                    server_ca_pem: p.tls.ca.read(false)?,
                    client_certificate_pem: p.tls.certificate.read(false)?,
                    client_key_pem: p.tls.key.read(true)?,
                },
            })
        } else {
            None
        };
        let identity = canonical::digest(
            "RX-HOST-INSTALLATION-CONFIG-v1",
            &(
                &config.installation,
                &config.host,
                &config.backend,
                config.bindings.sha256,
            ),
        )?;
        if let Backend::MelsecPackage { .. } = &config.backend {
            let package = super::device_package::load(&config.backend)?;
            if package.profile.installation != config.installation {
                return Err("device package belongs to another installation".into());
            }
            package.validate_bindings(&bindings)?;
        }
        if let Backend::JtcPackage { .. } = &config.backend {
            let package = super::jtc_package::load(&config.backend)?;
            if package.profile.installation != config.installation {
                return Err("JTC package belongs to another installation".into());
            }
            package.validate_bindings(&bindings)?;
        }
        Ok(Self {
            config,
            bindings,
            tls,
            publisher,
            identity,
        })
    }
}
