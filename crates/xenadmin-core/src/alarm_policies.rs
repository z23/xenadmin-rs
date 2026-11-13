use crate::{Error, HostId, ResourceTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use xenadmin_xapi::records::PoolRecord;

pub(crate) const KEY: &str = "perfmon";
const MAX_XML: usize = 262_144;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlarmMetric {
    Cpu,
    Network,
    Disk,
    FreeHostMemory,
    ControlDomainMemory,
    StorageThroughput,
}
impl AlarmMetric {
    pub const ALL: [Self; 6] = [
        Self::Cpu,
        Self::Network,
        Self::Disk,
        Self::FreeHostMemory,
        Self::ControlDomainMemory,
        Self::StorageThroughput,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU usage",
            Self::Network => "Network throughput",
            Self::Disk => "Disk throughput",
            Self::FreeHostMemory => "Free host memory",
            Self::ControlDomainMemory => "Control-domain memory usage",
            Self::StorageThroughput => "Storage throughput per host",
        }
    }
    pub fn unit(self) -> &'static str {
        match self {
            Self::Cpu | Self::ControlDomainMemory => "fraction (0–1)",
            Self::Network | Self::Disk => "bytes per second",
            Self::FreeHostMemory => "KiB",
            Self::StorageThroughput => "MiB per second",
        }
    }
    pub fn low_threshold(self) -> bool {
        self == Self::FreeHostMemory
    }
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Cpu => "cpu_usage",
            Self::Network => "network_usage",
            Self::Disk => "disk_usage",
            Self::FreeHostMemory => "memory_free_kib",
            Self::ControlDomainMemory => "mem_usage",
            Self::StorageThroughput => "sr_io_throughput_total_per_host",
        }
    }
    fn from_wire(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.wire() == name)
    }
    pub fn supported(self, target: ResourceTarget, dom0: Option<HostId>) -> bool {
        match target {
            ResourceTarget::Vm(_) => {
                if dom0.is_some() {
                    self == Self::ControlDomainMemory
                } else {
                    matches!(self, Self::Cpu | Self::Network | Self::Disk)
                }
            }
            ResourceTarget::Host(_) => {
                matches!(self, Self::Cpu | Self::Network | Self::FreeHostMemory)
            }
            ResourceTarget::Storage(_) => self == Self::StorageThroughput,
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmRule {
    pub metric: AlarmMetric,
    pub threshold: String,
    pub duration_seconds: u32,
    pub repeat_seconds: u32,
}
impl AlarmRule {
    pub fn validate(&self) -> Result<(), Error> {
        let value = self
            .threshold
            .parse::<f64>()
            .map_err(|_| Error::InvalidResourceCommand)?;
        if self.threshold.len() > 64
            || self.threshold.trim() != self.threshold
            || !value.is_finite()
            || value < 0.0
            || value > 1e18
            || matches!(
                self.metric,
                AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory
            ) && value > 1.0
            || !(1..=604_800).contains(&self.duration_seconds)
            || !(1..=2_592_000).contains(&self.repeat_seconds)
        {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmPolicy {
    pub target: ResourceTarget,
    pub control_domain_host: Option<HostId>,
    pub revision: String,
    pub rules: Vec<AlarmRule>,
    pub customized: Vec<AlarmMetric>,
    pub other_variables: usize,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmPolicyChange {
    pub expected_revision: String,
    pub control_domain_host: Option<HostId>,
    pub metric: AlarmMetric,
    pub rule: Option<AlarmRule>,
}
impl AlarmPolicyChange {
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        if self.expected_revision.len() != 64
            || !self
                .expected_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || !self.metric.supported(target, self.control_domain_host)
        {
            return Err(Error::InvalidResourceCommand);
        }
        if let Some(rule) = &self.rule {
            rule.validate()?;
            if rule.metric != self.metric {
                return Err(Error::InvalidResourceCommand);
            }
        }
        Ok(())
    }
}
pub(crate) fn raw(record: &Value) -> Result<Option<&str>, Error> {
    let map = record["other_config"]
        .as_object()
        .filter(|m| m.len() <= 4096)
        .ok_or(Error::Protocol("alarm metadata unavailable"))?;
    map.get(KEY)
        .map(|v| {
            v.as_str()
                .filter(|v| v.len() <= MAX_XML)
                .ok_or(Error::Protocol("invalid alarm configuration"))
        })
        .transpose()
}
fn revision(raw: Option<&str>) -> String {
    let mut hash = Sha256::new();
    hash.update([u8::from(raw.is_some())]);
    if let Some(raw) = raw {
        hash.update(raw.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
fn document(raw: &str) -> Result<roxmltree::Document<'_>, Error> {
    let doc = roxmltree::Document::parse_with_options(
        raw,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 8192,
        },
    )
    .map_err(|_| Error::Protocol("alarm XML is invalid or exceeds inspection limits"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "config"
        || root.tag_name().namespace().is_some()
        || root.attributes().len() != 0
        || root
            .children()
            .filter(|n| n.is_element())
            .any(|n| n.tag_name().name() != "variable" || n.tag_name().namespace().is_some())
    {
        return Err(Error::Protocol("unsupported alarm XML structure"));
    }
    for node in root.descendants().filter(|n| n.is_element() && *n != root) {
        let tag = node.tag_name().name();
        if tag == "variable" && node.parent() != Some(root)
            || matches!(
                tag,
                "name"
                    | "rrd_regex"
                    | "consolidation_fn"
                    | "alarm_trigger_level"
                    | "alarm_trigger_period"
                    | "alarm_auto_inhibit_period"
                    | "alarm_trigger_sense"
                    | "alarm_priority"
            ) && node.parent().is_none_or(|parent| {
                parent.parent() != Some(root) || parent.tag_name().name() != "variable"
            })
        {
            return Err(Error::Protocol(
                "nested alarm definitions cannot be edited safely",
            ));
        }
    }
    let mut names = BTreeSet::new();
    for node in root.children().filter(|n| n.is_element()) {
        let names_in_node = node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "name")
            .collect::<Vec<_>>();
        if names_in_node.len() != 1
            || !names.insert(attribute(names_in_node[0])?.to_owned())
            || names.len() > 128
        {
            return Err(Error::Protocol("duplicate or excessive alarm variables"));
        }
    }
    Ok(doc)
}
fn attribute<'a, 'input>(node: roxmltree::Node<'a, 'input>) -> Result<&'a str, Error> {
    if node.tag_name().namespace().is_some()
        || node.attributes().len() != 1
        || node
            .children()
            .any(|n| n.is_element() || n.is_text() && !n.text().unwrap_or("").trim().is_empty())
    {
        return Err(Error::Protocol("unsupported alarm parameter"));
    }
    node.attribute("value")
        .filter(|s| s.len() <= 4096)
        .ok_or(Error::Protocol("alarm parameter value unavailable"))
}
fn field<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Result<Option<&'a str>, Error> {
    let mut fields = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let first = fields.next();
    if fields.next().is_some() {
        return Err(Error::Protocol("duplicate alarm parameter"));
    }
    first.map(attribute).transpose()
}
fn rule(node: roxmltree::Node<'_, '_>, metric: AlarmMetric) -> Result<(AlarmRule, bool), Error> {
    let default_threshold = (metric == AlarmMetric::ControlDomainMemory).then_some("0.95");
    let threshold = field(node, "alarm_trigger_level")?
        .or(default_threshold)
        .ok_or(Error::Protocol("alarm threshold unavailable"))?;
    let r = AlarmRule {
        metric,
        threshold: threshold.into(),
        duration_seconds: field(node, "alarm_trigger_period")?
            .unwrap_or("60")
            .parse()
            .map_err(|_| Error::InvalidResourceCommand)?,
        repeat_seconds: field(node, "alarm_auto_inhibit_period")?
            .unwrap_or("3600")
            .parse()
            .map_err(|_| Error::InvalidResourceCommand)?,
    };
    r.validate()?;
    let customized = field(node, "rrd_regex")?.is_some()
        || field(node, "consolidation_fn")?.is_some()
        || field(node, "alarm_trigger_sense")?.is_some_and(|v| {
            v != if metric.low_threshold() {
                "low"
            } else {
                "high"
            }
        });
    Ok((r, customized))
}
pub(crate) fn project(target: ResourceTarget, dom0: Option<HostId>, record: &Value) -> AlarmPolicy {
    let mut p = AlarmPolicy {
        target,
        control_domain_host: dom0,
        revision: String::new(),
        rules: vec![],
        customized: vec![],
        other_variables: 0,
        error: None,
    };
    let result = (|| {
        let raw = raw(record)?;
        p.revision = revision(raw);
        let Some(raw) = raw.filter(|v| !v.trim().is_empty()) else {
            return Ok::<_, Error>(());
        };
        let doc = document(raw)?;
        for node in doc.root_element().children().filter(|n| n.is_element()) {
            let name = field(node, "name")?.unwrap();
            if let Some(metric) = AlarmMetric::from_wire(name).filter(|m| m.supported(target, dom0))
            {
                let (r, customized) = rule(node, metric)?;
                p.rules.push(r);
                if customized {
                    p.customized.push(metric);
                }
            } else {
                p.other_variables += 1;
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        p.error = Some(e.to_string());
    }
    p
}
fn rule_xml(rule: &AlarmRule) -> String {
    format!(
        "<alarm_trigger_level value=\"{}\"/><alarm_trigger_period value=\"{}\"/><alarm_auto_inhibit_period value=\"{}\"/>",
        rule.threshold, rule.duration_seconds, rule.repeat_seconds
    )
}
pub(crate) fn changed_xml(
    target: ResourceTarget,
    record: &Value,
    change: &AlarmPolicyChange,
) -> Result<String, Error> {
    change.validate(target)?;
    let original = raw(record)?;
    if revision(original) != change.expected_revision {
        return Err(Error::ResourceConflict);
    }
    let raw = original
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("<config/>");
    let doc = document(raw)?;
    let root = doc.root_element();
    let selected = root
        .children()
        .filter(|n| n.is_element())
        .find(|n| field(*n, "name").ok().flatten() == Some(change.metric.wire()));
    let replacement = if let Some(value) = &change.rule {
        let mut content = format!(
            "<variable><name value=\"{}\"/>{}",
            change.metric.wire(),
            rule_xml(value)
        );
        if let Some(node) = selected {
            if rule(node, change.metric)?.1 {
                return Err(Error::Protocol(
                    "custom alarm source or direction must be preserved",
                ));
            }
            if node.attributes().len() != 0 {
                return Err(Error::Protocol(
                    "custom alarm variable attributes cannot be edited",
                ));
            }
            for child in node.children().filter(|n| n.is_element()) {
                if !matches!(
                    child.tag_name().name(),
                    "name"
                        | "alarm_trigger_level"
                        | "alarm_trigger_period"
                        | "alarm_auto_inhibit_period"
                ) {
                    content.push_str(&raw[child.range()]);
                }
            }
        }
        content.push_str("</variable>");
        content
    } else {
        String::new()
    };
    let mut result = raw.to_owned();
    if let Some(node) = selected {
        result.replace_range(node.range(), &replacement);
    } else if change.rule.is_none() {
        return Err(Error::InvalidResourceCommand);
    } else {
        let root_text = &raw[root.range()];
        if let Some(end) = root_text.rfind("</") {
            result.insert_str(root.range().start + end, &replacement);
        } else {
            result.replace_range(root.range(), &format!("<config>{replacement}</config>"));
        }
    }
    if result.len() > MAX_XML {
        return Err(Error::InvalidResourceCommand);
    }
    document(&result)?;
    Ok(result)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmEmailSettings {
    pub destination: String,
    pub relay: String,
    pub language: String,
}
impl AlarmEmailSettings {
    pub fn validate(&self) -> Result<(), Error> {
        if self.destination.len() > 254
            || self.destination.starts_with('-')
            || self
                .destination
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
            || !self.destination.is_empty()
                && (self.destination.matches('@').count() != 1
                    || self.destination.split('@').any(str::is_empty))
            || self.relay.len() > 253
            || self
                .relay
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\' | '='))
            || !matches!(self.language.as_str(), "en-US" | "ja-JP" | "zh-CN")
        {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlarmEmailConfiguration {
    pub revision: String,
    pub settings: Option<AlarmEmailSettings>,
    pub error: Option<String>,
}
const EMAIL_KEYS: [&str; 3] = ["mail-destination", "ssmtp-mailhub", "mail-language"];
fn email_values(record: &Value) -> Result<Vec<Option<String>>, Error> {
    let map = PoolRecord::new(record)
        .other_config()
        .entries()
        .filter(|m| m.len() <= 4096)
        .ok_or(Error::Protocol(
            "pool alarm notification settings unavailable",
        ))?;
    EMAIL_KEYS
        .iter()
        .map(|key| {
            map.get(*key)
                .map(|v| {
                    v.as_str()
                        .filter(|v| v.len() <= 4096)
                        .map(str::to_owned)
                        .ok_or(Error::Protocol("invalid pool alarm notification setting"))
                })
                .transpose()
        })
        .collect()
}
fn email_revision(values: &[Option<String>]) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(values).expect("string options serialize"))
    )
}
pub(crate) fn project_email(record: &Value) -> AlarmEmailConfiguration {
    let mut result = AlarmEmailConfiguration {
        revision: String::new(),
        settings: None,
        error: None,
    };
    let parsed = (|| {
        let values = email_values(record)?;
        result.revision = email_revision(&values);
        let settings = AlarmEmailSettings {
            destination: values[0].clone().unwrap_or_default(),
            relay: values[1].clone().unwrap_or_default(),
            language: values[2].clone().unwrap_or_else(|| "en-US".into()),
        };
        settings.validate()?;
        result.settings = Some(settings);
        Ok::<(), Error>(())
    })();
    if let Err(error) = parsed {
        result.error = Some(error.to_string());
    }
    result
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlarmOperation {
    SetPolicy(AlarmPolicyChange),
    SetEmail {
        expected_revision: String,
        settings: AlarmEmailSettings,
    },
    Refresh {
        hosts: Vec<HostId>,
    },
}
impl AlarmOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SetPolicy(_) => "Set performance alarm",
            Self::SetEmail { .. } => "Set pool alarm email",
            Self::Refresh { .. } => "Request alarm configuration refresh",
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        match self {
            Self::SetPolicy(change) => change.validate(target),
            Self::SetEmail {
                expected_revision,
                settings,
            } if matches!(target, ResourceTarget::Pool(_)) => {
                if expected_revision.len() != 64
                    || !expected_revision.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(Error::InvalidResourceCommand);
                }
                settings.validate()
            }
            Self::Refresh { hosts }
                if matches!(target, ResourceTarget::Pool(_))
                    && !hosts.is_empty()
                    && hosts.len() <= 64
                    && hosts.iter().collect::<BTreeSet<_>>().len() == hosts.len() =>
            {
                Ok(())
            }
            _ => Err(Error::InvalidResourceCommand),
        }
    }
}
pub(crate) fn changed_email(
    record: &Value,
    expected: &str,
    settings: &AlarmEmailSettings,
) -> Result<Value, Error> {
    settings.validate()?;
    if email_revision(&email_values(record)?) != expected {
        return Err(Error::ResourceConflict);
    }
    let mut map = PoolRecord::new(record)
        .other_config()
        .entries()
        .cloned()
        .ok_or(Error::ResourceConflict)?;
    for (key, value) in
        EMAIL_KEYS
            .iter()
            .zip([&settings.destination, &settings.relay, &settings.language])
    {
        if value.is_empty() {
            map.remove(*key);
        } else {
            map.insert((*key).into(), Value::String(value.clone()));
        }
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn target() -> ResourceTarget {
        ResourceTarget::Host(uuid::Uuid::from_u128(1).into())
    }
    fn record(xml: Option<&str>) -> Value {
        match xml {
            Some(xml) => {
                json!({"other_config":{"perfmon":xml,"untouched":"private-unrelated-value"}})
            }
            None => json!({"other_config":{}}),
        }
    }
    fn change(record: &Value) -> AlarmPolicyChange {
        AlarmPolicyChange {
            expected_revision: project(target(), None, record).revision,
            control_domain_host: None,
            metric: AlarmMetric::Cpu,
            rule: Some(AlarmRule {
                metric: AlarmMetric::Cpu,
                threshold: "0.75".into(),
                duration_seconds: 120,
                repeat_seconds: 3600,
            }),
        }
    }
    #[test]
    fn alarm_edit_preserves_unrelated_variables_and_extra_parameters_without_exposing_xml() {
        let raw = "<config><variable><name value=\"cpu_usage\"/><alarm_trigger_level value=\"0.5\"/><alarm_priority value=\"2\"/></variable><variable><name value=\"custom-sensor\"/><rrd_regex value=\"custom.*\"/></variable></config>";
        let r = record(Some(raw));
        let updated = changed_xml(target(), &r, &change(&r)).unwrap();
        assert!(updated.contains("<alarm_priority value=\"2\"/>"));
        assert!(updated.contains(
            "<variable><name value=\"custom-sensor\"/><rrd_regex value=\"custom.*\"/></variable>"
        ));
        let p = project(target(), None, &record(Some(&updated)));
        assert_eq!(p.rules[0].threshold, "0.75");
        assert_eq!(p.other_variables, 1);
        let public = serde_json::to_string(&p).unwrap();
        assert!(!public.contains("rrd_regex"));
        assert!(!public.contains("private-unrelated-value"));
    }
    #[test]
    fn alarm_edit_refuses_stale_duplicate_oversized_external_and_custom_source_configs() {
        for raw in [
            "<config><variable><name value=\"cpu_usage\"/><name value=\"network_usage\"/></variable></config>",
            "<!DOCTYPE config [<!ENTITY data SYSTEM 'file:///tmp/missing'>]><config>&data;</config>",
            "<other/>",
        ] {
            assert!(project(target(), None, &record(Some(raw))).error.is_some());
        }
        let r = record(Some(
            "<config><variable><name value=\"cpu_usage\"/><alarm_trigger_level value=\"0.4\"/><rrd_regex value=\"other\"/></variable></config>",
        ));
        assert_eq!(
            project(target(), None, &r).customized,
            vec![AlarmMetric::Cpu]
        );
        assert!(changed_xml(target(), &r, &change(&r)).is_err());
        let empty = record(None);
        let mut c = change(&empty);
        c.expected_revision = "0".repeat(64);
        assert!(changed_xml(target(), &empty, &c).is_err());
        assert!(
            project(target(), None, &record(Some(&"x".repeat(MAX_XML + 1))))
                .error
                .is_some()
        );
    }
    #[test]
    fn alarm_nested_host_parameters_are_not_misclassified_as_plain_thresholds() {
        for nested in [
            "<extra><rrd_regex value=\"custom.*\"/></extra>",
            "<extra><name value=\"disk_usage\"/></extra>",
            "<extra><alarm_trigger_level value=\"0.99\"/></extra>",
            "<extra><variable><name value=\"other\"/></variable></extra>",
            "<extra><alarm_priority value=\"1\"/></extra>",
        ] {
            let raw = format!(
                "<config><variable><name value=\"cpu_usage\"/>{nested}<alarm_trigger_level value=\"0.5\"/></variable></config>"
            );
            let r = record(Some(&raw));
            assert!(project(target(), None, &r).error.is_some());
            assert!(changed_xml(target(), &r, &change(&r)).is_err());
        }
    }
    #[test]
    fn alarm_email_rejects_option_and_line_injection_and_preserves_unmanaged_keys() {
        let base = AlarmEmailSettings {
            destination: "operator@example.invalid".into(),
            relay: "relay.example.invalid:25".into(),
            language: "en-US".into(),
        };
        for destination in ["-C/tmp/file", "a@b\nCc:c@d", "a@b c@d", "missing-at"] {
            let mut value = base.clone();
            value.destination = destination.into();
            assert!(value.validate().is_err());
        }
        let r = serde_json::json!({"other_config":{"ssmtp-authpass":"private","unrelated":"keep"}});
        let before = project_email(&r);
        let changed = changed_email(&r, &before.revision, &base).unwrap();
        assert_eq!(changed["ssmtp-authpass"], "private");
        assert_eq!(changed["unrelated"], "keep");
        assert!(changed_email(&r, &"0".repeat(64), &base).is_err());
    }
    #[test]
    fn alarm_enable_disable_and_units_preserve_explicit_absence() {
        for raw in [None, Some(""), Some("<config/>"), Some("<config></config>")] {
            let r = record(raw);
            let c = change(&r);
            let updated = changed_xml(target(), &r, &c).unwrap();
            let r = record(Some(&updated));
            let mut c = change(&r);
            c.rule = None;
            let removed = changed_xml(target(), &r, &c).unwrap();
            assert!(
                project(target(), None, &record(Some(&removed)))
                    .rules
                    .is_empty()
            );
        }
        for value in ["NaN", "inf", "-1", "1.01", " 0.5", "0.5\" evil=\"1"] {
            let mut r = change(&record(None)).rule.unwrap();
            r.threshold = value.into();
            assert!(r.validate().is_err());
        }
        assert!(AlarmMetric::FreeHostMemory.low_threshold());
        assert!(!AlarmMetric::Cpu.low_threshold());
    }
}
