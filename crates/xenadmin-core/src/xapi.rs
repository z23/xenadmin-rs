use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use xenadmin_xapi::{
    SetField, TextField, Textual, WholeField,
    enumerations::{vm_operations, vm_power_state},
    records::{HostRecord, NetworkRecord, PoolRecord, SrRecord, TaskRecord, VmRecord},
};

use crate::{
    ConnectionOptions, Credentials, Error, Host, HostId, Inventory, Network, PoolId, PowerState,
    Storage, Vm, VmId,
};

#[path = "xapi_inventory.rs"]
mod inventory;
#[path = "xapi_management_topology.rs"]
mod management_topology;
pub(crate) use inventory::decode_server_message;
pub(crate) use management_topology::ManagementTopologyRecords;
#[cfg(test)]
#[path = "xapi_inventory_tests.rs"]
pub(crate) mod inventory_tests;

const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_OBJECTS: usize = 100_000;
pub(crate) const CLASSES: &[&str] = &["pool", "host", "VM", "SR", "network"];
const OPTIONAL_CLASSES: &[&str] = &[
    "VDI",
    "VBD",
    "VIF",
    "PIF",
    "PIF_metrics",
    "PBD",
    "Bond",
    "VLAN",
    "Tunnel",
    "Network_sriov",
    "SDN_controller",
    "PCI",
    "VM_guest_metrics",
    "VM_metrics",
    "host_metrics",
    "task",
    "message",
    "VM_appliance",
    "VMSS",
    "SM",
];

pub(crate) struct Session {
    permissions: crate::SessionPermissions,
    options: ConnectionOptions,
    commands: reqwest::Client,
    events: reqwest::Client,
    token: Zeroizing<String>,
    next_id: Arc<AtomicU64>,
    inventory_classes: std::sync::Mutex<BTreeMap<String, crate::model::ClassAvailability>>,
}

impl Session {
    pub(crate) fn permissions(&self) -> &crate::SessionPermissions {
        &self.permissions
    }
    pub(crate) fn options(&self) -> &ConnectionOptions {
        &self.options
    }
    pub(crate) fn session_cookie(&self) -> &str {
        &self.token
    }
    pub(crate) async fn login(
        options: &ConnectionOptions,
        credentials: &Credentials,
    ) -> Result<Self, Error> {
        Self::login_inner(options, credentials, true, None).await
    }
    pub(crate) async fn login_exact(
        options: &ConnectionOptions,
        credentials: &Credentials,
    ) -> Result<Self, Error> {
        Self::login_inner(options, credentials, false, None).await
    }
    pub(crate) async fn login_bound(
        options: &ConnectionOptions,
        credentials: &Credentials,
        first_hop: &crate::CertificateFingerprint,
    ) -> Result<Self, Error> {
        if !options.endpoint.as_str().starts_with("https://") {
            return Err(Error::InvalidEndpoint);
        }
        Self::login_inner(options, credentials, true, Some(first_hop)).await
    }
    async fn login_inner(
        options: &ConnectionOptions,
        credentials: &Credentials,
        follow_coordinator: bool,
        first_hop: Option<&crate::CertificateFingerprint>,
    ) -> Result<Self, Error> {
        let mut target = options.clone();
        let mut visited = std::collections::BTreeSet::new();
        for hop in 0..5 {
            if !visited.insert(target.endpoint.to_string()) {
                return Err(Error::Protocol("coordinator redirect loop"));
            }
            let commands =
                target.client_with_peer_identity(if hop == 0 { first_hop } else { None })?;
            let events =
                target.client_with_peer_identity(if hop == 0 { first_hop } else { None })?;
            let mut session = Self {
                permissions: crate::SessionPermissions::default(),
                options: target.clone(),
                commands,
                events,
                token: Zeroizing::new(String::new()),
                next_id: Arc::new(AtomicU64::new(1)),
                inventory_classes: std::sync::Mutex::new(BTreeMap::new()),
            };
            let result = session
                .rpc(
                    false,
                    "session.login_with_password",
                    vec![
                        json!(credentials.username),
                        json!(&*credentials.password),
                        json!("1.0"),
                        json!("xenadmin-rs"),
                    ],
                )
                .await;
            let result = match result {
                Err(Error::Coordinator(endpoint)) if follow_coordinator => {
                    target = options.for_endpoint(endpoint);
                    continue;
                }
                Err(Error::Certificate) => return Err(Error::EndpointCertificate(target.endpoint)),
                other => other?,
            };
            let token = result
                .as_str()
                .filter(|s| {
                    s.starts_with("OpaqueRef:")
                        && s.bytes().all(|b| b.is_ascii_graphic() && b != b';')
                })
                .ok_or(Error::Protocol("invalid session reference"))?;
            session.token = Zeroizing::new(token.into());
            session.permissions = match session
                .rpc(
                    false,
                    "session.get_record",
                    vec![json!(&*session.token), json!(&*session.token)],
                )
                .await
            {
                Ok(record) => crate::SessionPermissions::decode(&record),
                Err(Error::Api(code))
                    if matches!(
                        code.as_str(),
                        "SESSION_INVALID"
                            | "SESSION_AUTHENTICATION_FAILED"
                            | "SESSION_AUTHORIZATION_FAILED"
                    ) =>
                {
                    return Err(Error::Api(code));
                }
                Err(
                    error @ (Error::Coordinator(_)
                    | Error::Certificate
                    | Error::EndpointCertificate(_)
                    | Error::Closed),
                ) => return Err(error),
                Err(Error::Api(code))
                    if matches!(
                        code.as_str(),
                        "MESSAGE_METHOD_UNKNOWN"
                            | "MESSAGE_PARAMETER_COUNT_MISMATCH"
                            | "NOT_IMPLEMENTED"
                    ) =>
                {
                    crate::SessionPermissions::unknown(crate::PermissionUnknown::Unsupported)
                }
                Err(Error::Api(code))
                    if matches!(
                        code.as_str(),
                        "RBAC_PERMISSION_DENIED" | "PERMISSION_DENIED"
                    ) =>
                {
                    crate::SessionPermissions::unknown(crate::PermissionUnknown::DiscoveryDenied)
                }
                Err(_) => crate::SessionPermissions::unknown(crate::PermissionUnknown::Unavailable),
            };
            return Ok(session);
        }
        Err(Error::Protocol("too many coordinator redirects"))
    }

    async fn rpc_envelope(
        &self,
        event: bool,
        method: &str,
        params: Vec<Value>,
    ) -> Result<Value, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let client = if event { &self.events } else { &self.commands };
        // Debug builds can record a call whose arguments are not of the API's types.
        #[cfg(debug_assertions)]
        xenadmin_xapi::audit::observe_call(method, &params);
        let mut response = client
            .post(self.options.endpoint.rpc_url())
            .json(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .send()
            .await
            .map_err(|error| match transport_error(error) {
                Error::Certificate => Error::EndpointCertificate(self.options.endpoint.clone()),
                other => other,
            })?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(Error::Protocol("response exceeds the size limit"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(Error::Protocol("response exceeds the size limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = parse_rpc_json(&bytes)?;
        if value.get("id").and_then(Value::as_u64) != Some(id)
            || value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        {
            return Err(Error::Protocol("RPC response identity does not match"));
        }
        Ok(value)
    }

    async fn rpc(&self, event: bool, method: &str, params: Vec<Value>) -> Result<Value, Error> {
        let value = self.rpc_envelope(event, method, params).await?;
        let result = Self::rpc_result(value);
        if let Err(error) = &result
            && rpc_trace_enabled()
        {
            // Development diagnostics only: method name and sanitized error, never parameters.
            eprintln!("xenadmin rpc trace: {method} -> {error}");
        }
        result
    }
    fn rpc_result(value: Value) -> Result<Value, Error> {
        if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
            let code = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN_API_ERROR");
            let safe_code = safe_error_code(code);
            if safe_code == "HOST_IS_SLAVE"
                && let Some(host) = error
                    .get("data")
                    .and_then(Value::as_array)
                    .and_then(|v| v.first())
                    .and_then(Value::as_str)
                && let Ok(endpoint) = crate::Endpoint::parse(host)
            {
                return Err(Error::Coordinator(endpoint));
            }
            return Err(Error::Api(safe_code.into()));
        }
        value
            .get("result")
            .cloned()
            .ok_or(Error::Protocol("missing RPC result"))
    }

    pub(crate) async fn call(&self, method: &str, parameters: Vec<Value>) -> Result<Value, Error> {
        self.permissions.guard(method)?;
        let mut params = vec![json!(&*self.token)];
        params.extend(parameters);
        self.rpc(false, method, params).await
    }

    pub(crate) async fn storage_probe_xml(
        &self,
        parameters: Vec<Value>,
        expected_errors: &[&str],
    ) -> Result<String, Error> {
        self.permissions.guard("SR.probe")?;
        let mut params = vec![json!(&*self.token)];
        params.extend(parameters);
        let value = self.rpc_envelope(false, "SR.probe", params).await?;
        if value["error"]["message"]
            .as_str()
            .is_some_and(|code| expected_errors.contains(&code))
        {
            // These drivers return discovery XML in the missing-parameter error's stderr.
            // Raw backend errors remain private and are never added to Error or logs.
            let xml = value["error"]["data"]
                .as_array()
                .filter(|v| v.len() == 3)
                .and_then(|v| v[2].as_str())
                .filter(|v| v.len() <= 512 * 1024)
                .ok_or(Error::Protocol("storage discovery payload unavailable"))?;
            return Ok(xml.into());
        }
        Self::rpc_result(value)?
            .as_str()
            .filter(|v| v.len() <= 512 * 1024)
            .map(str::to_owned)
            .ok_or(Error::Protocol("invalid storage probe response"))
    }

    pub(crate) async fn events(&self, token: &str) -> Result<EventBatch, Error> {
        use crate::model::ClassAvailability;
        let mut states = self
            .inventory_classes
            .lock()
            .map_err(|_| Error::Protocol("inventory capability state unavailable"))?
            .clone();
        for class in OPTIONAL_CLASSES {
            states
                .entry(class.to_ascii_lowercase())
                .or_insert(ClassAvailability::Available);
        }
        let requested = || {
            CLASSES
                .iter()
                .copied()
                .chain(OPTIONAL_CLASSES.iter().copied().filter(|class| {
                    matches!(
                        states.get(&class.to_ascii_lowercase()),
                        Some(ClassAvailability::Available)
                    )
                }))
                .collect::<Vec<_>>()
        };
        let mut batch = match self.read_events(token, &requested()).await {
            Ok(batch) => batch,
            Err(Error::Api(code))
                if optional_class_error(&code).is_some()
                    && states
                        .values()
                        .any(|state| matches!(state, ClassAvailability::Available)) =>
            {
                for class in OPTIONAL_CLASSES {
                    let state = match self.call(&format!("{class}.get_all_records"), vec![]).await {
                        Ok(records) if records.is_object() => ClassAvailability::Available,
                        Ok(_) => ClassAvailability::InvalidData,
                        Err(Error::Api(code)) => {
                            optional_class_error(&code).unwrap_or(ClassAvailability::Unknown)
                        }
                        Err(error) => return Err(error),
                    };
                    states.insert(class.to_ascii_lowercase(), state);
                }
                let classes: Vec<_> = CLASSES
                    .iter()
                    .copied()
                    .chain(OPTIONAL_CLASSES.iter().copied().filter(|class| {
                        matches!(
                            states.get(&class.to_ascii_lowercase()),
                            Some(ClassAvailability::Available)
                        )
                    }))
                    .collect();
                let mut batch = self.read_events("", &classes).await?;
                batch.reset = true;
                batch
            }
            Err(error) => return Err(error),
        };
        batch.classes = states.clone();
        *self
            .inventory_classes
            .lock()
            .map_err(|_| Error::Protocol("inventory capability state unavailable"))? = states;
        Ok(batch)
    }

    async fn read_events(&self, token: &str, classes: &[&str]) -> Result<EventBatch, Error> {
        let value = self
            .rpc(
                true,
                "event.from",
                vec![
                    json!(&*self.token),
                    json!(classes),
                    json!(token),
                    json!(25.0),
                ],
            )
            .await?;
        let batch: EventBatch =
            serde_json::from_value(value).map_err(|_| Error::Protocol("invalid event batch"))?;
        if rpc_trace_enabled() && !batch.events.is_empty() {
            // Development diagnostics only: how many events of each class and kind came,
            // and over how many objects. Never a record, a value or a reference.
            let mut seen =
                BTreeMap::<(String, &str), (usize, std::collections::BTreeSet<&str>)>::new();
            for event in &batch.events {
                let entry = seen
                    .entry((
                        event.class_name.to_ascii_lowercase(),
                        event.operation.as_str(),
                    ))
                    .or_default();
                entry.0 += 1;
                entry.1.insert(event.reference.as_str());
            }
            let summary = seen
                .iter()
                .map(|((class, operation), (count, objects))| {
                    format!("{class} {operation} {count}/{}", objects.len())
                })
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!(
                "xenadmin event trace: {} events ({summary}){}",
                batch.events.len(),
                if token.is_empty() {
                    ", first batch"
                } else {
                    ""
                }
            );
        }
        Ok(batch)
    }

    pub(crate) async fn logout(&self) {
        let _ = self.call("session.logout", vec![]).await;
    }
    pub(crate) async fn logout_migration_receiver(&self, token: &str) {
        if token != self.session_cookie() && token.starts_with("OpaqueRef:") {
            let _ = self.rpc(false, "session.logout", vec![json!(token)]).await;
        }
    }

    pub(crate) async fn diagnose_power(
        &self,
        vm: VmId,
        action: crate::PowerAction,
    ) -> Result<crate::ActionAvailability, Error> {
        use crate::ActionAvailability;
        let reference = self
            .call("VM.get_by_uuid", vec![json!(vm.to_string())])
            .await?;
        let reference = opaque_reference(&reference)?;
        let record = self.call("VM.get_record", vec![json!(&reference)]).await?;
        let view = VmRecord::new(&record);
        if view.uuid().as_str() != Some(vm.to_string().as_str())
            || *view.is_a_snapshot() != false
            || *view.is_a_template() != false
            || *view.is_control_domain() != false
        {
            return Err(Error::Stale);
        }
        let advertised = inventory::power_availability(&record, action);
        if matches!(advertised, ActionAvailability::Blocked { .. }) {
            return Ok(advertised);
        }
        match self
            .call(
                "VM.assert_operation_valid",
                vec![json!(reference), json!(action.wire())],
            )
            .await
        {
            Ok(_) => Ok(ActionAvailability::Available),
            Err(Error::Api(code)) => Ok(match code.as_str() {
                "RBAC_PERMISSION_DENIED" | "PERMISSION_DENIED" => {
                    ActionAvailability::PermissionDenied
                }
                "MESSAGE_METHOD_UNKNOWN" | "MESSAGE_METHOD_UNAVAILABLE" => {
                    ActionAvailability::Unknown
                }
                "VM_BAD_POWER_STATE" => ActionAvailability::Unavailable,
                "OTHER_OPERATION_IN_PROGRESS"
                    if matches!(advertised, ActionAvailability::Busy { .. }) =>
                {
                    advertised
                }
                _ => ActionAvailability::Rejected { code },
            }),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn prepare_placement(
        &self,
        vm: VmId,
        host: HostId,
        action: crate::PowerAction,
    ) -> Result<(String, String), Error> {
        let (operation, required_state) = match action {
            crate::PowerAction::Start => ("start_on", vm_power_state::HALTED),
            crate::PowerAction::Resume => ("resume_on", vm_power_state::SUSPENDED),
            _ => return Err(Error::UnavailableAction),
        };
        let reference = self
            .call("VM.get_by_uuid", vec![json!(vm.to_string())])
            .await?;
        let reference = opaque_reference(&reference)?;
        let record = self.call("VM.get_record", vec![json!(&reference)]).await?;
        let view = VmRecord::new(&record);
        if view.uuid().as_str() != Some(vm.to_string().as_str())
            || *view.is_a_snapshot() != false
            || *view.is_a_template() != false
            || *view.is_control_domain() != false
        {
            return Err(Error::Stale);
        }
        if *view.power_state() != required_state || !advertises(&record, operation) {
            return Err(Error::UnavailableAction);
        }
        let destination = self
            .call("host.get_by_uuid", vec![json!(host.to_string())])
            .await?;
        let destination = opaque_reference(&destination)?;
        let destination_record = self
            .call("host.get_record", vec![json!(&destination)])
            .await?;
        let destination_view = HostRecord::new(&destination_record);
        if destination_view.uuid().as_str() != Some(host.to_string().as_str()) {
            return Err(Error::Stale);
        }
        if *destination_view.enabled() != true {
            return Err(Error::UnavailableAction);
        }
        self.call(
            "VM.assert_operation_valid",
            vec![json!(&reference), json!(operation)],
        )
        .await?;
        self.call(
            "VM.assert_can_boot_here",
            vec![json!(&reference), json!(&destination)],
        )
        .await?;
        Ok((reference, destination))
    }
}

/// XCP-ng 8.3.0 writes non-finite floats into JSON-RPC replies as the bare tokens `NaN`,
/// `Infinity` and `-Infinity` (for example in `VM.get_data_sources` records), which strict
/// JSON rejects. Replace those tokens outside string literals with `null` before parsing so
/// callers see an absent value rather than a protocol failure. The bytes are otherwise
/// untouched and the replacement never happens inside strings.
pub(crate) fn parse_rpc_json(bytes: &[u8]) -> Result<Value, Error> {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        return Ok(value);
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push(b);
            i += 1;
            continue;
        }
        let rest = &bytes[i..];
        let token = [&b"-Infinity"[..], b"Infinity", b"NaN"]
            .into_iter()
            .find(|token| rest.starts_with(token))
            .filter(|token| {
                !rest
                    .get(token.len())
                    .is_some_and(|next| next.is_ascii_alphanumeric() || *next == b'_')
            });
        if let Some(token) = token {
            out.extend_from_slice(b"null");
            i += token.len();
            continue;
        }
        out.push(b);
        i += 1;
    }
    serde_json::from_slice(&out).map_err(|_| Error::Protocol("invalid JSON"))
}

/// `XENADMIN_DEV_RPC_TRACE=1` prints failed method names and sanitized error codes to
/// stderr for lab diagnosis. Parameters, session tokens and payloads are never printed.
pub(crate) fn rpc_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("XENADMIN_DEV_RPC_TRACE").is_some_and(|v| v == "1"))
}

fn optional_class_error(code: &str) -> Option<crate::model::ClassAvailability> {
    use crate::model::ClassAvailability;
    match code {
        "RBAC_PERMISSION_DENIED" | "PERMISSION_DENIED" => Some(ClassAvailability::PermissionDenied),
        "MESSAGE_METHOD_UNKNOWN"
        | "MESSAGE_METHOD_UNAVAILABLE"
        | "NOT_IMPLEMENTED"
        | "EVENT_SUBSCRIPTION_PARSE_FAILURE"
        | "EVENT_CLASS_INVALID" => Some(ClassAvailability::Unsupported),
        _ => None,
    }
}

pub(crate) fn safe_error_code(code: &str) -> &str {
    if code.len() <= 100
        && !code.is_empty()
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        code
    } else {
        "UNKNOWN_API_ERROR"
    }
}

pub(crate) fn transport_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout
    } else if std::error::Error::source(&error).is_some_and(|_| {
        let text = format!("{error:?}").to_ascii_lowercase();
        text.contains("certificate") || text.contains("unknownissuer")
    }) {
        Error::Certificate
    } else {
        Error::Transport
    }
}

#[derive(Deserialize)]
pub(crate) struct EventBatch {
    pub(crate) token: String,
    pub(crate) events: Vec<Event>,
    #[serde(skip)]
    reset: bool,
    #[serde(skip)]
    classes: BTreeMap<String, crate::model::ClassAvailability>,
}

impl EventBatch {
    /// Makes this batch replace what the store holds instead of adding to it: for a full
    /// batch read again because the inventory was found out of step with the hosts.
    pub(crate) fn rebuild(&mut self) {
        self.reset = true;
    }
}

#[derive(Deserialize)]
pub(crate) struct Event {
    #[serde(rename = "class")]
    class_name: String,
    operation: String,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    snapshot: Value,
}

#[derive(Default, Clone)]
pub(crate) struct Store {
    records: BTreeMap<String, BTreeMap<String, Value>>,
    revision: u64,
    classes: BTreeMap<String, crate::model::ClassAvailability>,
}

impl Store {
    pub(crate) fn network_fabric_inventory(
        records: BTreeMap<String, BTreeMap<String, Value>>,
    ) -> Result<Inventory, Error> {
        let classes = records
            .keys()
            .map(|c| (c.clone(), crate::ClassAvailability::Available))
            .collect();
        Self {
            records,
            classes,
            revision: 0,
        }
        .inventory()
    }

    pub(crate) fn allows(&self, vm: VmId, action: &crate::OperationAction) -> bool {
        self.class("vm")
            .find(|(_, value)| {
                VmRecord::new(value).uuid().as_str() == Some(vm.to_string().as_str())
            })
            .is_some_and(|(_, value)| {
                let view = VmRecord::new(value);
                *view.is_control_domain() != true
                    && (*view.is_a_template() != true
                        || matches!(action, crate::OperationAction::Lifecycle(_))
                        || matches!(action, crate::OperationAction::Settings(p) if p.is_template()))
                    && *view.is_a_snapshot() != true
                    && match action {
                        crate::OperationAction::Power(action) => advertises(value, action.wire()),
                        crate::OperationAction::Snapshot(crate::SnapshotOperation::Workflow {
                            operation,
                        }) => {
                            let source = operation.snapshot().and_then(|id| {
                                self.class("vm")
                                    .find(|(_, candidate)| {
                                        let candidate = VmRecord::new(candidate);
                                        candidate.uuid().as_str() == Some(id.to_string().as_str())
                                            && *candidate.is_a_snapshot() == true
                                            && *candidate.is_control_domain() == false
                                            && candidate.snapshot_of().as_str()
                                                == self.vm_reference(vm).as_deref()
                                    })
                                    .map(|(_, candidate)| candidate)
                            });
                            operation.allows(value, source)
                        }
                        crate::OperationAction::Recovery(operation) => operation.allows(value),
                        crate::OperationAction::Migration(_)
                        | crate::OperationAction::Lifecycle(_)
                        | crate::OperationAction::Device(_) => true,
                        crate::OperationAction::Settings(p) => p.target_guard(value).is_ok(),
                        crate::OperationAction::Snapshot(
                            crate::SnapshotOperation::Checkpoint { .. },
                        ) => advertises(value, vm_operations::CHECKPOINT),
                        crate::OperationAction::Placement(p) => {
                            view.allowed_operations().as_array().is_some_and(|ops| {
                                ops.iter().any(|op| {
                                    op == match p.placement {
                                        crate::PowerAction::Start => "start_on",
                                        crate::PowerAction::Resume => "resume_on",
                                        _ => "",
                                    }
                                })
                            })
                        }
                        crate::OperationAction::Snapshot(crate::SnapshotOperation::Create {
                            ..
                        }) => advertises(value, vm_operations::SNAPSHOT),
                        crate::OperationAction::Snapshot(crate::SnapshotOperation::Revert {
                            snapshot,
                        }) => {
                            *view.power_state() == vm_power_state::HALTED
                                && self.class("vm").any(|(reference, candidate)| {
                                    let snapshot_view = VmRecord::new(candidate);
                                    snapshot_view.uuid().as_str()
                                        == Some(snapshot.to_string().as_str())
                                        && *snapshot_view.is_a_snapshot() == true
                                        && snapshot_view.snapshot_of().as_str()
                                            == self.vm_reference(vm).as_deref()
                                        && reference != "OpaqueRef:NULL"
                                        && advertises(candidate, vm_operations::REVERT)
                                })
                        }
                        crate::OperationAction::Snapshot(
                            crate::SnapshotOperation::Clone { snapshot, .. }
                            | crate::SnapshotOperation::Delete { snapshot, .. },
                        ) => self.class("vm").any(|(_, candidate)| {
                            let snapshot_view = VmRecord::new(candidate);
                            snapshot_view.uuid().as_str() == Some(snapshot.to_string().as_str())
                                && *snapshot_view.is_a_snapshot() == true
                                && snapshot_view.snapshot_of().as_str()
                                    == self.vm_reference(vm).as_deref()
                                && match action {
                                    crate::OperationAction::Snapshot(
                                        crate::SnapshotOperation::Clone { .. },
                                    ) => advertises(candidate, vm_operations::CLONE),
                                    _ => can_delete_snapshot(candidate),
                                }
                        }),
                    }
            })
    }
    pub(crate) fn vm_reference(&self, id: VmId) -> Option<String> {
        self.class("vm")
            .find(|(_, value)| {
                VmRecord::new(value).uuid().as_str() == Some(id.to_string().as_str())
            })
            .map(|(reference, _)| reference.clone())
    }
    /// The hosts whose record here differs, in what the client decides by, from a direct
    /// read of all host records, or that only one of the two has. Events normally make
    /// the two agree within moments; see `ConnectionOptions::inventory_check_interval`.
    pub(crate) fn host_drift(&self, fresh: &Value) -> std::collections::BTreeSet<String> {
        let held: BTreeMap<&String, &Value> = self.class("host").collect();
        // A set is the same set in any order.
        let members = |set: xenadmin_xapi::SetField<'_>| {
            set.members().map(|all| {
                all.iter()
                    .map(Value::to_string)
                    .collect::<std::collections::BTreeSet<_>>()
            })
        };
        let mut drift = std::collections::BTreeSet::new();
        let Some(fresh) = fresh.as_object() else {
            return drift;
        };
        for (reference, record) in fresh {
            let same = held.get(reference).is_some_and(|mine| {
                let (mine, theirs) = (HostRecord::new(mine), HostRecord::new(record));
                mine.enabled() == theirs.enabled()
                    && mine.other_config() == theirs.other_config()
                    && mine.address() == theirs.address()
                    && members(mine.resident_vms()) == members(theirs.resident_vms())
            });
            if !same {
                drift.insert(reference.clone());
            }
        }
        drift.extend(
            held.keys()
                .filter(|reference| !fresh.contains_key(reference.as_str()))
                .map(|reference| (*reference).clone()),
        );
        drift
    }
    pub(crate) fn apply(&mut self, batch: &EventBatch) -> Result<Inventory, Error> {
        let mut next = self.clone();
        if batch.reset {
            next.records.clear();
        }
        next.classes.extend(batch.classes.clone());
        for (name, state) in &batch.classes {
            if matches!(
                state,
                crate::model::ClassAvailability::Unsupported
                    | crate::model::ClassAvailability::PermissionDenied
            ) {
                next.records.remove(name);
            }
        }
        for event in &batch.events {
            let class_name = event.class_name.to_ascii_lowercase();
            let optional = OPTIONAL_CLASSES
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&class_name));
            if !CLASSES
                .iter()
                .chain(OPTIONAL_CLASSES)
                .any(|name| name.eq_ignore_ascii_case(&class_name))
            {
                continue;
            }
            if optional
                && matches!(
                    next.classes.get(&class_name),
                    Some(
                        crate::ClassAvailability::PermissionDenied
                            | crate::ClassAvailability::Unsupported
                    )
                )
            {
                continue;
            }
            let records = next.records.entry(class_name).or_default();
            match event.operation.as_str() {
                "add" | "mod" => {
                    if opaque_reference(&json!(&event.reference)).is_err()
                        || !event.snapshot.is_object()
                    {
                        if optional {
                            // Retain invalidity until this object is corrected/deleted or the store is resynchronized.
                            records.insert(event.reference.clone(), Value::Null);
                            continue;
                        }
                        return Err(Error::Protocol("invalid inventory event"));
                    }
                    records.insert(event.reference.clone(), event.snapshot.clone());
                }
                "del" => {
                    records.remove(&event.reference);
                }
                _ if optional => {
                    records.insert(event.reference.clone(), Value::Null);
                }
                _ => return Err(Error::Protocol("unknown inventory event operation")),
            }
        }
        if next.records.values().map(BTreeMap::len).sum::<usize>() > MAX_OBJECTS {
            return Err(Error::Protocol("inventory exceeds the object limit"));
        }
        next.revision += 1;
        let inventory = next.inventory()?;
        *self = next;
        Ok(inventory)
    }

    fn class(&self, name: &str) -> impl Iterator<Item = (&String, &Value)> {
        self.records.get(name).into_iter().flat_map(|r| r.iter())
    }

    pub(crate) fn inventory(&self) -> Result<Inventory, Error> {
        let pools: Vec<_> = self.class("pool").collect();
        if pools.len() != 1 {
            return Err(Error::Protocol(
                "expected one pool in the initial inventory",
            ));
        }
        let pool = pools[0].1;
        let pool_id = PoolId::parse(required(PoolRecord::new(pool).uuid())?)
            .map_err(|_| Error::Protocol("invalid pool UUID"))?;
        let mut host_refs = BTreeMap::new();
        let mut hosts = BTreeMap::new();
        for (reference, value) in self.class("host") {
            let view = HostRecord::new(value);
            let id = HostId::parse(required(view.uuid())?)
                .map_err(|_| Error::Protocol("invalid host UUID"))?;
            host_refs.insert(reference.as_str(), id);
            hosts.insert(
                id,
                Host {
                    id,
                    name: label(view.name_label()),
                    address: string(view.address()),
                    enabled: HostRecord::new(value).enabled().as_bool().unwrap_or(false),
                    version: HostRecord::new(value).software_version()["product_version"]
                        .as_str()
                        .unwrap_or("Unknown")
                        .into(),
                },
            );
        }
        let coordinator = pool
            .get("master")
            .and_then(Value::as_str)
            .and_then(|r| host_refs.get(r))
            .copied();
        let mut vm_refs = BTreeMap::new();
        for (reference, value) in self
            .class("vm")
            .filter(|(_, value)| *VmRecord::new(value).is_a_snapshot() != true)
        {
            let view = VmRecord::new(value);
            vm_refs.insert(
                reference.as_str(),
                VmId::parse(required(view.uuid())?)
                    .map_err(|_| Error::Protocol("invalid VM UUID"))?,
            );
        }
        let mut vms = BTreeMap::new();
        let mut snapshots = crate::SnapshotInventory::default();
        for (_, value) in self.class("vm") {
            let view = VmRecord::new(value);
            if *view.is_a_snapshot() == true {
                let mut actions = Vec::new();
                if advertises(value, vm_operations::REVERT) {
                    actions.push(crate::SnapshotAction::Revert);
                }
                if can_delete_snapshot(value) {
                    actions.push(crate::SnapshotAction::Delete);
                }
                if advertises(value, vm_operations::CLONE) {
                    actions.push(crate::SnapshotAction::Clone);
                }
                snapshots.insert(crate::Snapshot {
                    id: crate::SnapshotId::parse(required(view.uuid())?)
                        .map_err(|_| Error::Protocol("invalid snapshot UUID"))?,
                    vm: view
                        .snapshot_of()
                        .as_str()
                        .and_then(|reference| vm_refs.get(reference))
                        .copied(),
                    name: label(view.name_label()),
                    description: string(view.name_description()),
                    created_at: view
                        .snapshot_time()
                        .as_str()
                        .filter(|time| !time.is_empty())
                        .map(str::to_owned),
                    power_state: decode_power_state(value)?,
                    allowed_actions: actions,
                });
                continue;
            }
            let id = VmId::parse(required(view.uuid())?)
                .map_err(|_| Error::Protocol("invalid VM UUID"))?;
            let power_state = decode_power_state(value)?;
            vms.insert(
                id,
                Vm {
                    can_snapshot: advertises(value, vm_operations::SNAPSHOT),
                    allowed_actions: crate::PowerAction::ALL
                        .into_iter()
                        .filter(|action| advertises(value, action.wire()))
                        .collect(),
                    id,
                    name: label(view.name_label()),
                    description: string(view.name_description()),
                    power_state,
                    host: value
                        .get("resident_on")
                        .and_then(Value::as_str)
                        .and_then(|r| host_refs.get(r))
                        .copied(),
                    vcpus: number(view.vcpus_at_startup(), "VM.VCPUs_at_startup")?,
                    memory_bytes: number(view.memory_static_max(), "VM.memory_static_max")?,
                    is_template: view.is_a_template().as_bool().unwrap_or(false),
                    is_control_domain: view.is_control_domain().as_bool().unwrap_or(false),
                },
            );
        }
        let mut storage = Vec::new();
        for (_, value) in self.class("sr") {
            let view = SrRecord::new(value);
            storage.push(Storage {
                uuid: required(view.uuid())?
                    .parse()
                    .map_err(|_| Error::Protocol("invalid SR UUID"))?,
                name: label(view.name_label()),
                kind: string(view.type_()),
                capacity_bytes: storage_number(view.physical_size(), "SR.physical_size")?,
                used_bytes: storage_number(view.physical_utilisation(), "SR.physical_utilisation")?,
            });
        }
        let mut networks = Vec::new();
        for (_, value) in self.class("network") {
            let view = NetworkRecord::new(value);
            networks.push(Network {
                uuid: required(view.uuid())?
                    .parse()
                    .map_err(|_| Error::Protocol("invalid network UUID"))?,
                name: label(view.name_label()),
                bridge: string(view.bridge()),
            });
        }
        Ok(Inventory {
            pool_id,
            pool_name: label(PoolRecord::new(pool).name_label()),
            coordinator,
            hosts,
            vms,
            snapshots,
            storage,
            networks,
            details: inventory::decode(self),
            revision: self.revision,
        })
    }
}

/// Whether `allowed`, a record's `allowed_operations` read through its own class's view,
/// includes `operation`. Only a VM also has operations blocked by an administrator; that
/// is checked where a VM is read.
pub(crate) fn offers(allowed: SetField<'_>, operation: &str) -> bool {
    allowed
        .as_array()
        .is_some_and(|operations| operations.iter().any(|value| value == operation))
}

/// Whether a VM offers `action` and no administrator has blocked it. (Given a record of
/// another class it asks only the first: no other class has blocked operations. Such a
/// record is better asked through `offers` and its own view.)
pub(crate) fn advertises(record: &Value, action: &str) -> bool {
    record["allowed_operations"]
        .as_array()
        .is_some_and(|operations| operations.iter().any(|value| value == action))
        && record["blocked_operations"].get(action).is_none()
}

fn can_delete_snapshot(record: &Value) -> bool {
    match VmRecord::new(record).power_state().as_str() {
        Some(vm_power_state::HALTED) => advertises(record, vm_operations::DESTROY),
        Some(vm_power_state::SUSPENDED) => advertises(record, vm_operations::HARD_SHUTDOWN),
        _ => false,
    }
}

pub(crate) async fn prepare_mutation(
    session: &Session,
    operation: &crate::OperationRecord,
    parent: String,
) -> Result<String, Error> {
    use crate::{OperationAction, SnapshotOperation};
    if matches!(operation.action, OperationAction::Power(_)) {
        return Ok(parent);
    }
    let record = session.call("VM.get_record", vec![json!(&parent)]).await?;
    let view = VmRecord::new(&record);
    if view.uuid().as_str() != Some(operation.vm.to_string().as_str())
        || *view.is_a_snapshot() != false
        || (*view.is_a_template() != false
            && !matches!(&operation.action, OperationAction::Settings(p) if p.is_template()))
        || *view.is_control_domain() != false
    {
        return Err(Error::Stale);
    }
    match &operation.action {
        OperationAction::Settings(patch) => {
            patch.validate()?;
            patch.target_guard(&record)?;
            patch.preflight_remote(session, &parent).await?;
            let record = patch.resolve_record(session, record).await?;
            patch.preflight(&record)?;
            Ok(parent)
        }
        OperationAction::Snapshot(SnapshotOperation::Checkpoint { name }) => {
            crate::SnapshotName::new(name)?;
            if !advertises(&record, vm_operations::CHECKPOINT) {
                return Err(Error::UnavailableAction);
            }
            Ok(parent)
        }
        OperationAction::Snapshot(SnapshotOperation::Create { name }) => {
            crate::SnapshotName::new(name)?;
            if !advertises(&record, vm_operations::SNAPSHOT) {
                return Err(Error::UnavailableAction);
            }
            Ok(parent)
        }
        OperationAction::Snapshot(SnapshotOperation::Revert { snapshot }) => {
            if *view.power_state() != vm_power_state::HALTED {
                return Err(Error::SnapshotRequiresHaltedVm);
            }
            let reference = session
                .call("VM.get_by_uuid", vec![json!(snapshot.to_string())])
                .await?;
            let reference = opaque_reference(&reference)?;
            let candidate = session
                .call("VM.get_record", vec![json!(&reference)])
                .await?;
            let candidate_view = VmRecord::new(&candidate);
            if candidate_view.uuid().as_str() != Some(snapshot.to_string().as_str())
                || *candidate_view.is_a_snapshot() != true
                || *candidate_view.is_control_domain() != false
                || candidate_view.snapshot_of().as_str() != Some(&parent)
            {
                return Err(Error::Stale);
            }
            if !advertises(&candidate, vm_operations::REVERT) {
                return Err(Error::UnavailableAction);
            }
            Ok(reference)
        }
        OperationAction::Recovery(_)
        | OperationAction::Migration(_)
        | OperationAction::Lifecycle(_)
        | OperationAction::Placement(_)
        | OperationAction::Device(_) => Err(Error::UnavailableAction),
        OperationAction::Snapshot(SnapshotOperation::Clone { snapshot, name, .. }) => {
            crate::SnapshotName::new(name)?;
            let (reference, candidate) =
                crate::snapshots::snapshot_record(session, operation.vm, *snapshot).await?;
            if !advertises(&candidate, vm_operations::CLONE) {
                return Err(Error::UnavailableAction);
            }
            Ok(reference)
        }
        OperationAction::Snapshot(
            SnapshotOperation::Delete { .. } | SnapshotOperation::Workflow { .. },
        ) => Err(Error::UnavailableAction),
        OperationAction::Power(_) => unreachable!(),
    }
}

pub(crate) fn opaque_reference(value: &Value) -> Result<String, Error> {
    value
        .as_str()
        .filter(|value| {
            value.starts_with("OpaqueRef:")
                && *value != "OpaqueRef:NULL"
                && value.len() > "OpaqueRef:".len()
                && value.len() <= 1024
                && value.bytes().all(|byte| byte.is_ascii_graphic())
        })
        .map(str::to_owned)
        .ok_or(Error::Protocol("invalid object reference"))
}

pub(crate) fn snapshot_task_result(task: &Value) -> Result<String, Error> {
    let xml = TaskRecord::new(task)
        .result()
        .value()
        .as_str()
        .filter(|value| value.len() <= 4096)
        .ok_or(Error::Protocol("missing or oversized snapshot result"))?;
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 16,
        },
    )
    .map_err(|_| Error::Protocol("invalid snapshot result XML"))?;
    let root = doc.root_element();
    let children: Vec<_> = root.children().filter(|node| node.is_element()).collect();
    if root.tag_name().name() != "value" || root.attributes().len() != 0 {
        return Err(Error::Protocol("invalid snapshot result value"));
    }
    let content = match children.as_slice() {
        [] => root.text(),
        [child]
            if child.tag_name().name() == "string"
                && child.attributes().len() == 0
                && !child.children().any(|node| node.is_element())
                && root
                    .children()
                    .filter(|node| node.is_text())
                    .all(|node| node.text().unwrap_or("").trim().is_empty()) =>
        {
            child.text()
        }
        _ => None,
    }
    .ok_or(Error::Protocol("invalid snapshot result value"))?;
    opaque_reference(&json!(content))
}

pub(crate) async fn verify_created_snapshot(
    session: &Session,
    vm: VmId,
    reference: &str,
) -> Result<crate::SnapshotId, Error> {
    let parent = session
        .call("VM.get_by_uuid", vec![json!(vm.to_string())])
        .await?;
    let parent = opaque_reference(&parent)?;
    let result = session
        .call("VM.get_record", vec![json!(reference)])
        .await?;
    let created = VmRecord::new(&result);
    let id = created
        .uuid()
        .as_str()
        .and_then(|value| crate::SnapshotId::parse(value).ok())
        .ok_or(Error::Protocol("invalid created snapshot UUID"))?;
    if *created.is_a_snapshot() != true
        || *created.is_control_domain() != false
        || created.snapshot_of().as_str() != Some(&parent)
        || id.to_string() == vm.to_string()
    {
        return Err(Error::Protocol(
            "snapshot result does not belong to the selected VM",
        ));
    }
    Ok(id)
}

fn decode_power_state(record: &Value) -> Result<PowerState, Error> {
    Ok(match required(VmRecord::new(record).power_state())? {
        vm_power_state::RUNNING => PowerState::Running,
        vm_power_state::HALTED => PowerState::Halted,
        vm_power_state::PAUSED => PowerState::Paused,
        vm_power_state::SUSPENDED => PowerState::Suspended,
        other => PowerState::Unknown(other.into()),
    })
}

// The readers below take one field of a record, as a record view gives it. A field the
// record does not carry is null there, which each reader treats as absent. A whole record
// is another type and cannot be passed.
fn required<'a>(value: impl Textual<'a>) -> Result<&'a str, Error> {
    value
        .text()
        .ok_or(Error::Protocol("missing required inventory field"))
}
fn string<'a>(value: impl Textual<'a>) -> String {
    value.text().unwrap_or_default().into()
}
fn label(name_label: TextField<'_>) -> String {
    name_label
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or("Unnamed")
        .into()
}
fn integer(value: WholeField<'_>, field: &'static str) -> Result<Option<i64>, Error> {
    let invalid = || Error::InventoryNumber {
        field,
        expected: "a signed 64-bit integer",
    };
    match value.value() {
        Value::Null => Ok(None),
        Value::Number(n) => n.as_i64().map(Some).ok_or_else(invalid),
        Value::String(n) => n.parse::<i64>().map(Some).map_err(|_| invalid()),
        _ => Err(invalid()),
    }
}
fn number(value: WholeField<'_>, field: &'static str) -> Result<u64, Error> {
    u64::try_from(integer(value, field)?.unwrap_or(0)).map_err(|_| Error::InventoryNumber {
        field,
        expected: "a non-negative integer",
    })
}
fn storage_number(value: WholeField<'_>, field: &'static str) -> Result<Option<u64>, Error> {
    match integer(value, field)? {
        None | Some(-1) => Ok(None),
        Some(value) => u64::try_from(value)
            .map(Some)
            .map_err(|_| Error::InventoryNumber {
                field,
                expected: "a non-negative integer or -1 for unknown",
            }),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_finite_bare_tokens_become_null_only_outside_strings() {
        let body = br#"{"jsonrpc":"2.0","id":1,"result":[{"name":"cpu","min":0.0,"max":Infinity,"value":NaN,"note":"Infinity NaN -Infinity","floor":-Infinity,"units":"NaNometer"}]}"#;
        let value = super::parse_rpc_json(body).unwrap();
        let record = &value["result"][0];
        assert_eq!(record["min"], 0.0);
        assert!(record["max"].is_null());
        assert!(record["value"].is_null());
        assert!(record["floor"].is_null());
        assert_eq!(record["note"], "Infinity NaN -Infinity");
        assert_eq!(record["units"], "NaNometer");
        assert!(super::parse_rpc_json(br#"{"a":Infinityx}"#).is_err());
        assert!(super::parse_rpc_json(b"not json").is_err());
        assert_eq!(
            super::parse_rpc_json(br#"{"escaped":"a\"Infinity\"b"}"#).unwrap()["escaped"],
            "a\"Infinity\"b"
        );
    }

    use super::*;
    #[test]
    fn snapshot_results_require_one_bounded_xml_value_and_a_non_null_reference() {
        for xml in [
            "<value>OpaqueRef:snapshot</value>",
            "<value><string>OpaqueRef:snapshot</string></value>",
        ] {
            assert_eq!(
                snapshot_task_result(&json!({"result":xml})).unwrap(),
                "OpaqueRef:snapshot"
            );
        }
        for xml in [
            "OpaqueRef:snapshot",
            "<value>OpaqueRef:NULL</value>",
            "<value>OpaqueRef:</value>",
            "<value><string>OpaqueRef:first</string><string>OpaqueRef:second</string></value>",
            "<value>extra<string>OpaqueRef:snapshot</string></value>",
            "<!DOCTYPE value [<!ENTITY ref 'OpaqueRef:secret'>]><value>&ref;</value>",
            &format!("<value>{}</value>", "x".repeat(4096)),
        ] {
            assert!(
                snapshot_task_result(&json!({"result":xml})).is_err(),
                "accepted malformed task result"
            );
        }
    }
    #[test]
    fn snapshot_inventory_rebuilds_relationships_atomically_and_excludes_snapshots_from_vms() {
        let mut store = Store::default();
        let mut batch = initial();
        batch.events.insert(0, serde_json::from_value(json!({"class":"VM","operation":"add","ref":"OpaqueRef:s","snapshot":{"uuid":"00000000-0000-0000-0000-000000000003","name_label":"Before update","is_a_snapshot":true,"is_a_template":true,"snapshot_of":"OpaqueRef:v","power_state":"Halted","allowed_operations":["revert"]}})).unwrap());
        let inventory = store.apply(&batch).unwrap();
        let vm = inventory.guests().next().unwrap().id;
        assert_eq!(inventory.vms.len(), 1);
        assert_eq!(inventory.snapshots.for_vm(vm).count(), 1);
        let serialized = serde_json::to_string(&inventory).unwrap();
        let decoded: Inventory = serde_json::from_str(&serialized).unwrap();
        assert_eq!(decoded.snapshots.for_vm(vm).count(), 1);
        let orphan = serde_json::from_value(
            json!({"token":"two","events":[{"class":"VM","operation":"del","ref":"OpaqueRef:v"}]}),
        )
        .unwrap();
        let inventory = store.apply(&orphan).unwrap();
        assert!(inventory.vms.is_empty());
        assert_eq!(inventory.snapshots.len(), 1);
        assert!(inventory.snapshots.iter().next().unwrap().vm.is_none());
        let deleted = serde_json::from_value(json!({"token":"three","events":[{"class":"VM","operation":"del","ref":"OpaqueRef:s"}]})).unwrap();
        assert!(store.apply(&deleted).unwrap().snapshots.is_empty());
    }
    fn initial() -> EventBatch {
        serde_json::from_value(json!({"token":"one", "events":[
            {"class":"pool","operation":"add","ref":"OpaqueRef:p","snapshot":{"uuid":"00000000-0000-0000-0000-000000000001","name_label":"Lab"}},
            {"class":"VM","operation":"add","ref":"OpaqueRef:v","snapshot":{"uuid":"00000000-0000-0000-0000-000000000002","name_label":"Guest","power_state":"Halted","VCPUs_at_startup":"2","memory_static_max":1073741824}}
        ]})).unwrap()
    }
    #[test]
    fn malformed_batch_does_not_partially_modify_inventory() {
        let mut store = Store::default();
        store.apply(&initial()).unwrap();
        let batch: EventBatch = serde_json::from_value(json!({"token":"two","events":[
            {"class":"VM","operation":"del","ref":"OpaqueRef:v"},
            {"class":"host","operation":"add","ref":"OpaqueRef:bad","snapshot":{"uuid":"not-a-uuid"}}
        ]})).unwrap();
        assert!(store.apply(&batch).is_err());
        let inventory = store.inventory().unwrap();
        assert_eq!(inventory.guests().count(), 1);
        assert_eq!(inventory.revision, 1);
    }
    #[test]
    fn storage_unknown_sizes_do_not_prevent_inventory_bootstrap() {
        for unknown in [json!(-1), json!("-1")] {
            let mut batch = initial();
            batch.events.push(
                serde_json::from_value(json!({
                    "class":"SR", "operation":"add", "ref":"OpaqueRef:sr",
                    "snapshot":{"uuid":"00000000-0000-0000-0000-000000000003",
                        "name_label":"Unscanned storage", "type":"iso",
                        "physical_size":unknown, "physical_utilisation":unknown}
                }))
                .unwrap(),
            );
            let mut store = Store::default();
            let inventory = store
                .apply(&batch)
                .expect("XAPI introduces SRs with -1 sizes");
            assert_eq!(inventory.guests().count(), 1);
            assert_eq!(inventory.storage.len(), 1);
            assert_eq!(inventory.storage[0].capacity_bytes, None);
            assert_eq!(inventory.storage[0].used_bytes, None);
            batch.events.last_mut().unwrap().snapshot["physical_size"] = json!("9007199254740993");
            batch.events.last_mut().unwrap().snapshot["physical_utilisation"] = json!(0);
            let inventory = store.apply(&batch).unwrap();
            assert_eq!(
                inventory.storage[0].capacity_bytes,
                Some(9_007_199_254_740_993)
            );
            assert_eq!(inventory.storage[0].used_bytes, Some(0));
            for invalid in [json!(-2), json!(0.5), json!("bad-size")] {
                batch.events.last_mut().unwrap().snapshot["physical_size"] = invalid;
                assert!(
                    store
                        .apply(&batch)
                        .unwrap_err()
                        .to_string()
                        .contains("SR.physical_size")
                );
                assert_eq!(store.inventory().unwrap().revision, 2);
            }
        }
    }
    #[test]
    fn malformed_numbers_identify_the_field_and_leave_inventory_unchanged() {
        let mut store = Store::default();
        store.apply(&initial()).unwrap();
        for invalid in [
            json!("not-a-number"),
            json!(1.5),
            json!(-2),
            json!("9223372036854775808"),
        ] {
            let mut batch = initial();
            batch.events[1].snapshot["memory_static_max"] = invalid;
            let error = store.apply(&batch).unwrap_err().to_string();
            assert!(error.contains("VM.memory_static_max"));
            assert!(!error.contains("not-a-number"));
            assert_eq!(store.inventory().unwrap().revision, 1);
        }
    }
    #[test]
    fn unknown_power_states_survive_and_deletes_remove_objects() {
        let mut store = Store::default();
        let mut batch = initial();
        batch.events[1].snapshot["power_state"] = json!("FutureState");
        let inventory = store.apply(&batch).unwrap();
        assert_eq!(
            inventory.guests().next().unwrap().power_state,
            PowerState::Unknown("FutureState".into())
        );
        let deletion = serde_json::from_value(
            json!({"token":"two","events":[{"class":"VM","operation":"del","ref":"OpaqueRef:v"}]}),
        )
        .unwrap();
        assert_eq!(store.apply(&deletion).unwrap().guests().count(), 0);
    }
}
