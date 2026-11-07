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

use crate::{
    ConnectionOptions, Credentials, Error, Host, HostId, Inventory, Network, PoolId, PowerState,
    Storage, Vm, VmId,
};

const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const MAX_OBJECTS: usize = 100_000;
pub(crate) const CLASSES: &[&str] = &["pool", "host", "VM", "SR", "network"];

pub(crate) struct Session {
    options: ConnectionOptions,
    commands: reqwest::Client,
    events: reqwest::Client,
    token: Zeroizing<String>,
    next_id: Arc<AtomicU64>,
}

impl Session {
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
        let mut target = options.clone();
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..5 {
            if !visited.insert(target.endpoint.to_string()) {
                return Err(Error::Protocol("coordinator redirect loop"));
            }
            let commands = target.client()?;
            let events = target.client()?;
            let mut session = Self {
                options: target.clone(),
                commands,
                events,
                token: Zeroizing::new(String::new()),
                next_id: Arc::new(AtomicU64::new(1)),
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
                Err(Error::Coordinator(endpoint)) => {
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
            return Ok(session);
        }
        Err(Error::Protocol("too many coordinator redirects"))
    }

    async fn rpc(&self, event: bool, method: &str, params: Vec<Value>) -> Result<Value, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let client = if event { &self.events } else { &self.commands };
        let mut response = client
            .post(self.options.endpoint.rpc_url())
            .json(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .send()
            .await
            .map_err(transport_error)?;
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
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| Error::Protocol("invalid JSON"))?;
        if value.get("id").and_then(Value::as_u64) != Some(id)
            || value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        {
            return Err(Error::Protocol("RPC response identity does not match"));
        }
        if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
            let code = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN_API_ERROR");
            let safe_code = if code.len() <= 100
                && code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            {
                code
            } else {
                "UNKNOWN_API_ERROR"
            };
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
        let mut params = vec![json!(&*self.token)];
        params.extend(parameters);
        self.rpc(false, method, params).await
    }

    pub(crate) async fn events(&self, token: &str) -> Result<EventBatch, Error> {
        let value = self
            .rpc(
                true,
                "event.from",
                vec![
                    json!(&*self.token),
                    json!(CLASSES),
                    json!(token),
                    json!(25.0),
                ],
            )
            .await?;
        serde_json::from_value(value).map_err(|_| Error::Protocol("invalid event batch"))
    }

    pub(crate) async fn logout(&self) {
        let _ = self.call("session.logout", vec![]).await;
    }
}

fn transport_error(error: reqwest::Error) -> Error {
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
}

impl Store {
    pub(crate) fn allows(&self, vm: VmId, action: crate::PowerAction) -> bool {
        self.class("vm")
            .find(|(_, value)| value["uuid"].as_str() == Some(vm.to_string().as_str()))
            .is_some_and(|(_, value)| {
                value["is_control_domain"] != true
                    && value["is_a_template"] != true
                    && value["is_a_snapshot"] != true
                    && value["allowed_operations"]
                        .as_array()
                        .is_some_and(|operations| operations.iter().any(|v| v == action.wire()))
            })
    }
    pub(crate) fn vm_reference(&self, id: VmId) -> Option<String> {
        self.class("vm")
            .find(|(_, value)| value["uuid"].as_str() == Some(id.to_string().as_str()))
            .map(|(reference, _)| reference.clone())
    }
    pub(crate) fn apply(&mut self, batch: &EventBatch) -> Result<Inventory, Error> {
        let mut next = self.clone();
        for event in &batch.events {
            let class_name = event.class_name.to_ascii_lowercase();
            if !["pool", "host", "vm", "sr", "network"].contains(&class_name.as_str()) {
                continue;
            }
            let records = next.records.entry(class_name).or_default();
            match event.operation.as_str() {
                "add" | "mod" => {
                    if !event.reference.starts_with("OpaqueRef:") || !event.snapshot.is_object() {
                        return Err(Error::Protocol("invalid inventory event"));
                    }
                    records.insert(event.reference.clone(), event.snapshot.clone());
                }
                "del" => {
                    records.remove(&event.reference);
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
        let pool_id = PoolId::parse(required(pool, "uuid")?)
            .map_err(|_| Error::Protocol("invalid pool UUID"))?;
        let mut host_refs = BTreeMap::new();
        let mut hosts = BTreeMap::new();
        for (reference, value) in self.class("host") {
            let id = HostId::parse(required(value, "uuid")?)
                .map_err(|_| Error::Protocol("invalid host UUID"))?;
            host_refs.insert(reference.as_str(), id);
            hosts.insert(
                id,
                Host {
                    id,
                    name: label(value),
                    address: string(value, "address"),
                    enabled: value["enabled"].as_bool().unwrap_or(false),
                    version: value["software_version"]["product_version"]
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
        let mut vms = BTreeMap::new();
        for (_, value) in self.class("vm") {
            let id = VmId::parse(required(value, "uuid")?)
                .map_err(|_| Error::Protocol("invalid VM UUID"))?;
            let power_state = match required(value, "power_state")? {
                "Running" => PowerState::Running,
                "Halted" => PowerState::Halted,
                "Paused" => PowerState::Paused,
                "Suspended" => PowerState::Suspended,
                other => PowerState::Unknown(other.into()),
            };
            vms.insert(
                id,
                Vm {
                    allowed_actions: [
                        crate::PowerAction::Start,
                        crate::PowerAction::CleanShutdown,
                        crate::PowerAction::Reboot,
                        crate::PowerAction::HardShutdown,
                        crate::PowerAction::HardReboot,
                    ]
                    .into_iter()
                    .filter(|action| {
                        value["allowed_operations"]
                            .as_array()
                            .is_some_and(|operations| operations.iter().any(|v| v == action.wire()))
                    })
                    .collect(),
                    id,
                    name: label(value),
                    description: string(value, "name_description"),
                    power_state,
                    host: value
                        .get("resident_on")
                        .and_then(Value::as_str)
                        .and_then(|r| host_refs.get(r))
                        .copied(),
                    vcpus: number(value, "VCPUs_at_startup")?,
                    memory_bytes: number(value, "memory_static_max")?,
                    is_template: value["is_a_template"].as_bool().unwrap_or(false),
                    is_snapshot: value["is_a_snapshot"].as_bool().unwrap_or(false),
                    is_control_domain: value["is_control_domain"].as_bool().unwrap_or(false),
                },
            );
        }
        let mut storage = Vec::new();
        for (_, value) in self.class("sr") {
            storage.push(Storage {
                uuid: required(value, "uuid")?
                    .parse()
                    .map_err(|_| Error::Protocol("invalid SR UUID"))?,
                name: label(value),
                kind: string(value, "type"),
                capacity_bytes: number(value, "physical_size")?,
                used_bytes: number(value, "physical_utilisation")?,
            });
        }
        let mut networks = Vec::new();
        for (_, value) in self.class("network") {
            networks.push(Network {
                uuid: required(value, "uuid")?
                    .parse()
                    .map_err(|_| Error::Protocol("invalid network UUID"))?,
                name: label(value),
                bridge: string(value, "bridge"),
            });
        }
        Ok(Inventory {
            pool_id,
            pool_name: label(pool),
            coordinator,
            hosts,
            vms,
            storage,
            networks,
            revision: self.revision,
        })
    }
}

fn required<'a>(record: &'a Value, key: &str) -> Result<&'a str, Error> {
    record
        .get(key)
        .and_then(Value::as_str)
        .ok_or(Error::Protocol("missing required inventory field"))
}
fn string(record: &Value, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}
fn label(record: &Value) -> String {
    record
        .get("name_label")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .unwrap_or("Unnamed")
        .into()
}
fn number(record: &Value, key: &str) -> Result<u64, Error> {
    match record.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or(Error::Protocol("invalid inventory number")),
        Some(Value::String(n)) => n
            .parse()
            .map_err(|_| Error::Protocol("invalid inventory number")),
        _ => Err(Error::Protocol("invalid inventory number")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
