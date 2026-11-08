use crate::{
    CertificateFingerprint, Credentials, Error, HostId, ObservedResource, ResourceAction, RoleId,
    SubjectId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryHostState {
    pub host: HostId,
    pub auth_type: String,
    pub service_name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolSecurityOperation {
    JoinDirectory {
        domain: String,
        expected_hosts: Vec<DirectoryHostState>,
    },
    LeaveDirectory {
        expected_hosts: Vec<DirectoryHostState>,
        remove_machine_accounts: bool,
    },
    AddSubject {
        identifier: String,
    },
    RemoveSubject {
        subject: SubjectId,
        expected_identifier: String,
        expected_roles: Vec<RoleId>,
    },
    ChangeSubjectRole {
        subject: SubjectId,
        expected_identifier: String,
        expected_roles: Vec<RoleId>,
        role: RoleId,
        grant: bool,
    },
    InstallCertificate {
        host: HostId,
        expected: CertificateFingerprint,
        fingerprint: CertificateFingerprint,
    },
    ResetCertificate {
        host: HostId,
        expected: CertificateFingerprint,
    },
}
impl PoolSecurityOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::JoinDirectory { .. } => "Join Active Directory",
            Self::LeaveDirectory { .. } => "Leave Active Directory",
            Self::AddSubject { .. } => "Grant directory subject access",
            Self::RemoveSubject { .. } => "Remove directory subject access",
            Self::ChangeSubjectRole { grant: true, .. } => "Grant subject role",
            Self::ChangeSubjectRole { grant: false, .. } => "Remove subject role",
            Self::InstallCertificate { .. } => "Install host TLS certificate",
            Self::ResetCertificate { .. } => "Reset host TLS certificate",
        }
    }
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::JoinDirectory { .. } => "pool.enable_external_auth",
            Self::LeaveDirectory { .. } => "pool.disable_external_auth",
            Self::AddSubject { .. } => "subject.create",
            Self::RemoveSubject { .. } => "subject.destroy",
            Self::ChangeSubjectRole { grant: true, .. } => "subject.add_to_roles",
            Self::ChangeSubjectRole { grant: false, .. } => "subject.remove_from_roles",
            Self::InstallCertificate { .. } => "host.install_server_certificate",
            Self::ResetCertificate { .. } => "host.reset_server_certificate",
        }
    }
    pub(crate) fn secret_required(&self) -> bool {
        matches!(
            self,
            Self::JoinDirectory { .. }
                | Self::InstallCertificate { .. }
                | Self::LeaveDirectory {
                    remove_machine_accounts: true,
                    ..
                }
        )
    }
    pub(crate) fn valid(&self) -> bool {
        let text = |s: &str| !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control);
        let roles = |ids: &[RoleId]| {
            ids.len() <= 64
                && ids.iter().collect::<std::collections::BTreeSet<_>>().len() == ids.len()
        };
        let hosts = |values: &[DirectoryHostState]| {
            !values.is_empty()
                && values.len() <= 256
                && values
                    .iter()
                    .map(|v| v.host)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == values.len()
                && values.iter().all(|v| {
                    v.auth_type.len() <= 64
                        && v.service_name.len() <= 1024
                        && !v
                            .auth_type
                            .chars()
                            .chain(v.service_name.chars())
                            .any(char::is_control)
                })
        };
        match self {
            Self::JoinDirectory {
                domain,
                expected_hosts,
            } => {
                domain.len() <= 253
                    && domain.split('.').all(|p| {
                        !p.is_empty()
                            && p.len() <= 63
                            && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                            && !p.starts_with('-')
                            && !p.ends_with('-')
                    })
                    && hosts(expected_hosts)
                    && expected_hosts.iter().all(|v| v.auth_type.is_empty())
            }
            Self::LeaveDirectory { expected_hosts, .. } => {
                hosts(expected_hosts) && expected_hosts.iter().all(|v| v.auth_type == "AD")
            }
            Self::AddSubject { identifier } => text(identifier),
            Self::RemoveSubject {
                expected_identifier,
                expected_roles,
                ..
            } => text(expected_identifier) && roles(expected_roles),
            Self::ChangeSubjectRole {
                expected_identifier,
                expected_roles,
                role,
                grant,
                ..
            } => {
                text(expected_identifier)
                    && roles(expected_roles)
                    && expected_roles.contains(role) != *grant
            }
            Self::InstallCertificate {
                expected,
                fingerprint,
                ..
            } => expected != fingerprint,
            Self::ResetCertificate { .. } => true,
        }
    }
}

/// Transient inputs deliberately implement neither serialization nor cloning.
pub enum PoolSensitiveChange {
    JoinDirectory {
        domain: String,
        expected_hosts: Vec<DirectoryHostState>,
        credentials: Credentials,
    },
    LeaveDirectory {
        expected_hosts: Vec<DirectoryHostState>,
        credentials: Credentials,
    },
    InstallCertificate {
        host: HostId,
        expected: CertificateFingerprint,
        certificate: ServerCertificate,
    },
}
impl std::fmt::Debug for PoolSensitiveChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PoolSensitiveChange([redacted])")
    }
}
pub struct ServerCertificate {
    pub(crate) certificate: String,
    pub(crate) private_key: Zeroizing<String>,
    pub(crate) chain: String,
    fingerprint: CertificateFingerprint,
}
impl std::fmt::Debug for ServerCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerCertificate([redacted])")
    }
}
impl ServerCertificate {
    pub fn new(certificate: String, private_key: String, chain: String) -> Result<Self, Error> {
        let private_key = Zeroizing::new(private_key);
        if certificate.len() > 256 * 1024
            || private_key.len() > 64 * 1024
            || chain.len() > 512 * 1024
            || !private_key.contains("-----BEGIN PRIVATE KEY-----")
        {
            return Err(Error::InvalidResourceCommand);
        }
        let leaf = certificate_pem(&certificate)?;
        if leaf.len() != 1 {
            return Err(Error::InvalidResourceCommand);
        }
        let items = rustls_pemfile::read_all(&mut private_key.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::InvalidResourceCommand)?;
        let [rustls_pemfile::Item::Pkcs8Key(key)] = items.as_slice() else {
            return Err(Error::InvalidResourceCommand);
        };
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(key.clone_key());
        let signer = rustls::crypto::ring::sign::any_supported_type(&key)
            .map_err(|_| Error::InvalidResourceCommand)?;
        rustls::sign::CertifiedKey::new(leaf.clone(), signer)
            .keys_match()
            .map_err(|_| Error::InvalidResourceCommand)?;
        let chain_certs = certificate_pem(&chain)?;
        if chain_certs.len() > 16 || (!chain.trim().is_empty() && chain_certs.is_empty()) {
            return Err(Error::InvalidResourceCommand);
        }
        let fingerprint = format!("{:x}", Sha256::digest(leaf[0].as_ref())).parse()?;
        Ok(Self {
            certificate,
            private_key,
            chain,
            fingerprint,
        })
    }
    pub fn fingerprint(&self) -> CertificateFingerprint {
        self.fingerprint.clone()
    }
}
fn certificate_pem(pem: &str) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, Error> {
    rustls_pemfile::read_all(&mut pem.as_bytes())
        .map(|item| match item {
            Ok(rustls_pemfile::Item::X509Certificate(certificate)) => Ok(certificate),
            _ => Err(Error::InvalidResourceCommand),
        })
        .collect()
}
pub(crate) enum SensitiveInputs {
    Credentials(Credentials),
    Certificate(ServerCertificate),
}
impl SensitiveInputs {
    pub(crate) fn valid_for(&self, op: &PoolSecurityOperation) -> bool {
        match (self, op) {
            (
                Self::Credentials(c),
                PoolSecurityOperation::JoinDirectory { .. }
                | PoolSecurityOperation::LeaveDirectory {
                    remove_machine_accounts: true,
                    ..
                },
            ) => {
                !c.username.is_empty()
                    && c.username.len() <= 1024
                    && !c.username.chars().any(char::is_control)
                    && !c.password.is_empty()
                    && c.password.len() <= 65536
                    && !c.password.contains('\0')
            }
            (
                Self::Certificate(cert),
                PoolSecurityOperation::InstallCertificate { fingerprint, .. },
            ) => &cert.fingerprint == fingerprint,
            _ => false,
        }
    }
}
impl PoolSensitiveChange {
    pub(crate) fn split(
        self,
        target: ObservedResource,
    ) -> (ObservedResource, ResourceAction, SensitiveInputs) {
        let (op, inputs) = match self {
            Self::JoinDirectory {
                domain,
                expected_hosts,
                credentials,
            } => (
                PoolSecurityOperation::JoinDirectory {
                    domain,
                    expected_hosts,
                },
                SensitiveInputs::Credentials(credentials),
            ),
            Self::LeaveDirectory {
                expected_hosts,
                credentials,
            } => (
                PoolSecurityOperation::LeaveDirectory {
                    expected_hosts,
                    remove_machine_accounts: true,
                },
                SensitiveInputs::Credentials(credentials),
            ),
            Self::InstallCertificate {
                host,
                expected,
                certificate,
            } => (
                PoolSecurityOperation::InstallCertificate {
                    host,
                    expected,
                    fingerprint: certificate.fingerprint(),
                },
                SensitiveInputs::Certificate(certificate),
            ),
        };
        (
            target,
            ResourceAction::Pool(crate::PoolOperation::Security(op)),
            inputs,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn certificate_requires_matching_single_pkcs8_key_and_bounded_pem() {
        let first = rcgen::generate_simple_self_signed(vec!["fixture.test".into()]).unwrap();
        let second = rcgen::generate_simple_self_signed(vec!["other.test".into()]).unwrap();
        let cert = ServerCertificate::new(
            first.cert.pem(),
            first.key_pair.serialize_pem(),
            String::new(),
        )
        .unwrap();
        assert!(!format!("{cert:?}").contains("BEGIN"));
        assert!(
            ServerCertificate::new(
                first.cert.pem(),
                second.key_pair.serialize_pem(),
                String::new()
            )
            .is_err()
        );
        assert!(
            ServerCertificate::new(
                first.cert.pem(),
                format!(
                    "{}{}",
                    first.key_pair.serialize_pem(),
                    first.key_pair.serialize_pem()
                ),
                String::new()
            )
            .is_err()
        );
        assert!(
            ServerCertificate::new(
                format!("{}{}", first.cert.pem(), first.cert.pem()),
                first.key_pair.serialize_pem(),
                String::new()
            )
            .is_err()
        );
        assert!(
            ServerCertificate::new(
                first.cert.pem(),
                first.key_pair.serialize_pem(),
                "x".repeat(512 * 1024 + 1)
            )
            .is_err()
        );
    }
    #[test]
    fn security_commands_refuse_ambiguous_hosts_and_role_changes() {
        let host = HostId::from(uuid::Uuid::new_v4());
        let state = DirectoryHostState {
            host,
            auth_type: String::new(),
            service_name: String::new(),
        };
        assert!(
            PoolSecurityOperation::JoinDirectory {
                domain: "example.test".into(),
                expected_hosts: vec![state.clone()]
            }
            .valid()
        );
        assert!(
            !PoolSecurityOperation::JoinDirectory {
                domain: "example.test".into(),
                expected_hosts: vec![state.clone(), state.clone()]
            }
            .valid()
        );
        assert!(
            !PoolSecurityOperation::JoinDirectory {
                domain: "bad-.test".into(),
                expected_hosts: vec![state]
            }
            .valid()
        );
        let role = RoleId::from(uuid::Uuid::new_v4());
        assert!(
            !PoolSecurityOperation::ChangeSubjectRole {
                subject: SubjectId::from(uuid::Uuid::new_v4()),
                expected_identifier: "S-fixture".into(),
                expected_roles: vec![role],
                role,
                grant: true
            }
            .valid()
        );
    }
}
