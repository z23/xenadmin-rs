use crate::{Error, ObservedVm, PoolId, VmId, xapi::Session};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use xenadmin_xapi::enumerations::{
    domain_type, pool_guest_secureboot_readiness, vm_secureboot_readiness,
};
use xenadmin_xapi::records::{PoolRecord, VmRecord};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootReadiness {
    Ready,
    ReadyWithoutRevocations,
    FirstBoot,
    Disabled,
    NotSupported,
    SetupMode,
    IncompleteCertificates,
    NotReady,
    Unavailable(String),
}
impl BootReadiness {
    pub fn explanation(&self) -> &str {
        match self {
            Self::Ready => "Certificate set ready; verify actual enforcement inside the guest",
            Self::ReadyWithoutRevocations => "Certificate set lacks the revocation database (dbx)",
            Self::FirstBoot => "Guest certificate state will be initialized on first boot",
            Self::Disabled => "Secure Boot policy is disabled",
            Self::NotSupported => "This firmware does not support Secure Boot",
            Self::SetupMode => "Setup Mode: no platform key; policy does not enforce Secure Boot",
            Self::IncompleteCertificates => "Guest certificates are incomplete",
            Self::NotReady => "Pool certificate defaults are not ready",
            Self::Unavailable(reason) => reason,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestBootStatus {
    pub firmware: Option<String>,
    pub policy: Option<bool>,
    pub guest: BootReadiness,
    pub pool: BootReadiness,
}
pub struct GuestBootStatusQuery {
    receiver: watch::Receiver<Option<Result<GuestBootStatus, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl GuestBootStatusQuery {
    pub fn latest(&self) -> Option<Result<GuestBootStatus, String>> {
        self.receiver.borrow().clone().or_else(|| {
            self.receiver
                .has_changed()
                .is_err()
                .then(|| Err(Error::Closed.to_string()))
        })
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            if receiver.changed().await.is_ok() {
                callback();
            }
        });
    }
}
impl Drop for GuestBootStatusQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedVm,
    sender: watch::Sender<Option<Result<GuestBootStatus, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedVm,
    runtime: tokio::runtime::Handle,
) -> (GuestBootStatusQuery, Request) {
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    (
        GuestBootStatusQuery {
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        Request {
            observed,
            sender,
            cancel,
        },
    )
}
impl Request {
    pub(crate) fn fail(self, error: Error) {
        self.sender.send_replace(Some(Err(error.to_string())));
    }
    pub(crate) async fn run(
        self,
        session: Arc<Session>,
        receiver: watch::Receiver<Arc<crate::ConnectionSnapshot>>,
        generation: CancellationToken,
    ) {
        let result = tokio::select! {_=self.cancel.cancelled()=>Err(Error::Closed),_=generation.cancelled()=>Err(Error::Stale),r=tokio::time::timeout(Duration::from_secs(30),read(&session,self.observed.pool,self.observed.id))=>r.unwrap_or(Err(Error::Timeout))};
        let current = receiver
            .borrow()
            .observe_vm(self.observed.id)
            .is_ok_and(|o| o == self.observed);
        self.sender.send_replace(Some(if current {
            result.map_err(|e| e.to_string())
        } else {
            Err(Error::Stale.to_string())
        }));
    }
}

pub(crate) fn is_hvm(r: &Value) -> bool {
    match VmRecord::new(r).domain_type().name() {
        Some(domain_type::HVM) => true,
        None | Some(domain_type::UNSPECIFIED) => VmRecord::new(r).hvm_boot_policy() == "BIOS order",
        _ => false,
    }
}
pub(crate) fn firmware(r: &Value) -> Option<&str> {
    if !is_hvm(r) {
        return None;
    }
    match VmRecord::new(r)
        .hvm_boot_params()
        .entries()?
        .get("firmware")
    {
        None => Some("bios"),
        Some(v) => v.as_str().filter(|v| matches!(*v, "bios" | "uefi")),
    }
}
pub(crate) fn policy(r: &Value) -> Option<bool> {
    match VmRecord::new(r).platform().entries()?.get("secureboot") {
        None => Some(false),
        Some(v) if v == "false" => Some(false),
        Some(v) if v == "true" => Some(true),
        _ => None,
    }
}
pub(crate) fn default_template(r: &Value) -> Option<bool> {
    if VmRecord::new(r).is_default_template() == true
        || VmRecord::new(r).other_config()["default_template"] == "true"
    {
        return Some(true);
    }
    if VmRecord::new(r).is_default_template() == false
        || VmRecord::new(r)
            .other_config()
            .entries()
            .is_some_and(|m| m.get("default_template").is_none_or(|v| v == "false"))
    {
        Some(false)
    } else {
        None
    }
}
pub(crate) fn custom_template(r: &Value) -> bool {
    VmRecord::new(r).is_a_template() == true && default_template(r) == Some(false)
}
pub(crate) async fn require_policy_support(
    session: &Session,
    reference: &str,
) -> Result<(), Error> {
    let value = session
        .call("VM.get_secureboot_readiness", vec![json!(reference)])
        .await?;
    if !matches!(
        value.as_str(),
        Some(
            vm_secureboot_readiness::READY
                | vm_secureboot_readiness::READY_NO_DBX
                | vm_secureboot_readiness::FIRST_BOOT
                | vm_secureboot_readiness::DISABLED
                | vm_secureboot_readiness::NOT_SUPPORTED
                | vm_secureboot_readiness::SETUP_MODE
                | vm_secureboot_readiness::CERTS_INCOMPLETE
        )
    ) {
        return Err(Error::UnsupportedVmLifecycle);
    }
    Ok(())
}
fn readiness(result: Result<Value, Error>) -> BootReadiness {
    match result {
        Ok(v) => match v.as_str() {
            Some(vm_secureboot_readiness::READY) => BootReadiness::Ready,
            Some(vm_secureboot_readiness::READY_NO_DBX) => BootReadiness::ReadyWithoutRevocations,
            Some(vm_secureboot_readiness::FIRST_BOOT) => BootReadiness::FirstBoot,
            Some(vm_secureboot_readiness::DISABLED) => BootReadiness::Disabled,
            Some(vm_secureboot_readiness::NOT_SUPPORTED) => BootReadiness::NotSupported,
            Some(vm_secureboot_readiness::SETUP_MODE) => BootReadiness::SetupMode,
            Some(vm_secureboot_readiness::CERTS_INCOMPLETE) => {
                BootReadiness::IncompleteCertificates
            }
            Some(pool_guest_secureboot_readiness::NOT_READY) => BootReadiness::NotReady,
            _ => BootReadiness::Unavailable("Host returned an unknown readiness value".into()),
        },
        Err(e) => BootReadiness::Unavailable(format!("Readiness unavailable: {e}")),
    }
}
pub(crate) async fn read(
    session: &Session,
    pool: PoolId,
    vm: VmId,
) -> Result<GuestBootStatus, Error> {
    let r = session
        .call("VM.get_by_uuid", vec![json!(vm.to_string())])
        .await?;
    let before = session.call("VM.get_record", vec![r.clone()]).await?;
    if VmRecord::new(&before).uuid() != vm.to_string()
        || VmRecord::new(&before).is_control_domain() != false
        || VmRecord::new(&before).is_a_snapshot() != false
    {
        return Err(Error::Stale);
    }
    let pr = session
        .call("pool.get_by_uuid", vec![json!(pool.to_string())])
        .await?;
    let p = session.call("pool.get_record", vec![pr.clone()]).await?;
    if PoolRecord::new(&p).uuid() != pool.to_string() {
        return Err(Error::Stale);
    }
    let guest = readiness(
        session
            .call("VM.get_secureboot_readiness", vec![r.clone()])
            .await,
    );
    let pool = readiness(
        session
            .call("pool.get_guest_secureboot_readiness", vec![pr])
            .await,
    );
    let after = session.call("VM.get_record", vec![r]).await?;
    if VmRecord::new(&before).uuid() != VmRecord::new(&after).uuid()
        || VmRecord::new(&before).is_a_template() != VmRecord::new(&after).is_a_template()
        || VmRecord::new(&after).is_control_domain() != false
        || VmRecord::new(&after).is_a_snapshot() != false
        || firmware(&before) != firmware(&after)
        || policy(&before) != policy(&after)
        || VmRecord::new(&before).nvram() != VmRecord::new(&after).nvram()
    {
        return Err(Error::Stale);
    }
    Ok(GuestBootStatus {
        firmware: firmware(&after).map(str::to_owned),
        policy: policy(&after),
        guest,
        pool,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xapi::inventory_tests::Fixture;
    #[tokio::test]
    async fn policy_and_certificate_readiness_are_independent_without_exposing_nvram() {
        for status in [
            "ready",
            "setup_mode",
            "first_boot",
            "unknown_value",
            "RBAC_PERMISSION_DENIED",
        ] {
            let f=Fixture::new(move|request| {
                let v=match request["method"].as_str().unwrap() {
                    "VM.get_by_uuid"=>json!("OpaqueRef:vm"),
                    "pool.get_by_uuid"=>json!("OpaqueRef:pool"),
                    "pool.get_record"=>json!({"uuid":uuid::Uuid::from_u128(1)}),
                    "VM.get_record"=>json!({"uuid":uuid::Uuid::from_u128(2),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"HVM_boot_policy":"BIOS order","HVM_boot_params":{"firmware":"uefi"},"platform":{"secureboot":"true"},"NVRAM":{"private":"secret-cert-bytes"}}),
                    "VM.get_secureboot_readiness" if status=="RBAC_PERMISSION_DENIED"=>return json!({"error":{"message":status,"data":["private-host-detail"]}}),
                    "VM.get_secureboot_readiness"=>json!(status),
                    "pool.get_guest_secureboot_readiness"=>json!("ready_no_dbx"),
                    "session.logout"=>Value::Null,
                    method=>panic!("unexpected RPC {method}"),
                };json!({"result":v})
            }).await;
            let result = read(
                &f.session().await,
                uuid::Uuid::from_u128(1).into(),
                uuid::Uuid::from_u128(2).into(),
            )
            .await
            .unwrap();
            assert_eq!(result.policy, Some(true));
            assert_eq!(result.pool, BootReadiness::ReadyWithoutRevocations);
            match status {
                "ready" => assert_eq!(result.guest, BootReadiness::Ready),
                "setup_mode" => assert_eq!(result.guest, BootReadiness::SetupMode),
                "first_boot" => assert_eq!(result.guest, BootReadiness::FirstBoot),
                _ => assert!(matches!(result.guest, BootReadiness::Unavailable(_))),
            }
            assert!(!format!("{result:?}").contains("secret-cert"));
            assert!(!format!("{result:?}").contains("private-host"));
        }
    }
    #[tokio::test]
    async fn changed_policy_during_readiness_query_is_rejected() {
        let reads = std::sync::atomic::AtomicUsize::new(0);
        let f=Fixture::new(move|r| {
            let v=match r["method"].as_str().unwrap() {
                "VM.get_by_uuid"=>json!("OpaqueRef:vm"),"pool.get_by_uuid"=>json!("OpaqueRef:pool"),
                "pool.get_record"=>json!({"uuid":uuid::Uuid::from_u128(1)}),
                "VM.get_record"=>json!({"uuid":uuid::Uuid::from_u128(2),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"HVM_boot_policy":"BIOS order","HVM_boot_params":{"firmware":"uefi"},"platform":{"secureboot":if reads.fetch_add(1,std::sync::atomic::Ordering::SeqCst)==0 {"true"}else{"false"}}}),
                "VM.get_secureboot_readiness"|"pool.get_guest_secureboot_readiness"=>json!("ready"),"session.logout"=>Value::Null,_=>panic!("unexpected query")
            };json!({"result":v})
        }).await;
        assert!(matches!(
            read(
                &f.session().await,
                uuid::Uuid::from_u128(1).into(),
                uuid::Uuid::from_u128(2).into()
            )
            .await,
            Err(Error::Stale)
        ));
    }
}
