use crate::{Error, HostId, ResourceTarget, SnapshotPolicy, SnapshotPolicyId, VmId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::enumerations::{vmss_frequency, vmss_type};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduledSnapshotKind {
    Disk,
    Checkpoint,
}
impl ScheduledSnapshotKind {
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Disk => vmss_type::SNAPSHOT,
            Self::Checkpoint => vmss_type::CHECKPOINT,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotSchedule {
    Hourly { minute: u8 },
    Daily { hour: u8, minute: u8 },
    Weekly { hour: u8, minute: u8, days: Vec<u8> },
}
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
impl SnapshotSchedule {
    pub fn validate(&self) -> Result<(), Error> {
        let (hour, minute, days) = match self {
            Self::Hourly { minute } => (0, *minute, None),
            Self::Daily { hour, minute } => (*hour, *minute, None),
            Self::Weekly { hour, minute, days } => (*hour, *minute, Some(days)),
        };
        if hour < 24
            && [0, 15, 30, 45].contains(&minute)
            && days.is_none_or(|v| {
                !v.is_empty()
                    && v.len() <= 7
                    && v.iter().all(|d| *d < 7)
                    && v.windows(2).all(|pair| pair[0] < pair[1])
            })
        {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
    pub fn frequency(&self) -> &'static str {
        match self {
            Self::Hourly { .. } => vmss_frequency::HOURLY,
            Self::Daily { .. } => vmss_frequency::DAILY,
            Self::Weekly { .. } => vmss_frequency::WEEKLY,
        }
    }
    pub fn fields(&self) -> BTreeMap<String, String> {
        let (hour, minute, days) = match self {
            Self::Hourly { minute } => (None, *minute, None),
            Self::Daily { hour, minute } => (Some(*hour), *minute, None),
            Self::Weekly { hour, minute, days } => (Some(*hour), *minute, Some(days)),
        };
        let mut m = BTreeMap::from([("min".into(), minute.to_string())]);
        if let Some(h) = hour {
            m.insert("hour".into(), h.to_string());
        }
        if let Some(d) = days {
            let mut d = d.clone();
            d.sort_unstable();
            m.insert(
                "days".into(),
                d.iter()
                    .filter_map(|d| DAYS.get(*d as usize))
                    .copied()
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        m
    }
    pub(crate) fn decode(frequency: &str, m: &BTreeMap<String, String>) -> Result<Self, Error> {
        let n = |key: &str| {
            m.get(key)
                .and_then(|s| s.parse::<u8>().ok())
                .ok_or(Error::InvalidResourceCommand)
        };
        let mut v = match frequency {
            vmss_frequency::HOURLY => Self::Hourly { minute: n("min")? },
            vmss_frequency::DAILY => Self::Daily {
                hour: n("hour")?,
                minute: n("min")?,
            },
            vmss_frequency::WEEKLY => Self::Weekly {
                hour: n("hour")?,
                minute: n("min")?,
                days: m
                    .get("days")
                    .ok_or(Error::InvalidResourceCommand)?
                    .split(',')
                    .map(|s| {
                        DAYS.iter()
                            .position(|d| *d == s)
                            .map(|i| i as u8)
                            .ok_or(Error::InvalidResourceCommand)
                    })
                    .collect::<Result<_, _>>()?,
            },
            _ => return Err(Error::InvalidResourceCommand),
        };
        if let Self::Weekly { days, .. } = &mut v {
            days.sort_unstable();
        }
        v.validate()?;
        if v.fields().keys().ne(m.keys()) {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(v)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPolicyConfig {
    pub name: String,
    pub description: String,
    pub kind: ScheduledSnapshotKind,
    pub retained: u8,
    pub schedule: SnapshotSchedule,
}
impl SnapshotPolicyConfig {
    pub fn validate(&self) -> Result<(), Error> {
        self.schedule.validate()?;
        if self.name.trim().is_empty()
            || self.name.chars().count() > 255
            || self.name.chars().any(char::is_control)
            || self.description.len() > 16_384
            || self.description.contains('\0')
            || !(1..=10).contains(&self.retained)
        {
            Err(Error::InvalidResourceCommand)
        } else {
            Ok(())
        }
    }
    pub fn from_policy(p: &SnapshotPolicy) -> Result<Self, Error> {
        let v = Self {
            name: p.name.clone(),
            description: p.description.clone(),
            kind: match p.kind.as_str() {
                vmss_type::SNAPSHOT => ScheduledSnapshotKind::Disk,
                vmss_type::CHECKPOINT => ScheduledSnapshotKind::Checkpoint,
                _ => return Err(Error::UnavailableAction),
            },
            retained: p
                .retained_snapshots
                .and_then(|n| n.try_into().ok())
                .ok_or(Error::InvalidResourceCommand)?,
            schedule: SnapshotSchedule::decode(&p.frequency, &p.schedule)?,
        };
        v.validate()?;
        Ok(v)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPolicyState {
    pub id: SnapshotPolicyId,
    pub config: SnapshotPolicyConfig,
    pub enabled: bool,
    pub members: Vec<VmId>,
}
impl SnapshotPolicyState {
    pub fn from_policy(p: &SnapshotPolicy) -> Result<Self, Error> {
        if !p.vms_known {
            return Err(Error::UnavailableAction);
        }
        Ok(Self {
            id: p.id,
            config: SnapshotPolicyConfig::from_policy(p)?,
            enabled: p.enabled.ok_or(Error::UnavailableAction)?,
            members: p.vms.clone(),
        })
    }
    fn validate(&self) -> Result<(), Error> {
        self.config.validate()?;
        if self.members.len() > 256
            || self.members.iter().collect::<BTreeSet<_>>().len() != self.members.len()
        {
            Err(Error::InvalidResourceCommand)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPolicyClock {
    pub coordinator: HostId,
    pub utc_offset_minutes: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotPolicyOperation {
    Create {
        config: SnapshotPolicyConfig,
        clock: SnapshotPolicyClock,
    },
    Configure {
        expected: SnapshotPolicyState,
        config: SnapshotPolicyConfig,
        clock: SnapshotPolicyClock,
    },
    SetEnabled {
        expected: SnapshotPolicyState,
        enabled: bool,
        clock: Option<SnapshotPolicyClock>,
    },
    Membership {
        expected: SnapshotPolicyState,
        vm: VmId,
        join: bool,
    },
    DeleteEmpty {
        expected: SnapshotPolicyState,
    },
    RunNow {
        expected: SnapshotPolicyState,
    },
}
impl SnapshotPolicyOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Create snapshot schedule",
            Self::Configure { .. } => "Configure snapshot schedule",
            Self::SetEnabled { enabled: true, .. } => "Enable snapshot schedule",
            Self::SetEnabled { .. } => "Disable snapshot schedule",
            Self::Membership { join: true, .. } => "Assign VM to snapshot schedule",
            Self::Membership { .. } => "Remove VM from snapshot schedule",
            Self::DeleteEmpty { .. } => "Delete empty snapshot schedule",
            Self::RunNow { .. } => "Run snapshot schedule now",
        }
    }
    /// Create and destroy run as host tasks. `VMSS.snapshot_now` has no asynchronous form
    /// on XCP-ng 8.3.0 ("No async mode for this operation"), so a run is a synchronous call
    /// whose reply carries the plugin result.
    pub(crate) fn asynchronous(&self) -> bool {
        matches!(self, Self::Create { .. } | Self::DeleteEmpty { .. })
    }
    pub(crate) fn expected(&self) -> Option<&SnapshotPolicyState> {
        match self {
            Self::Create { .. } => None,
            Self::Configure { expected, .. }
            | Self::SetEnabled { expected, .. }
            | Self::Membership { expected, .. }
            | Self::DeleteEmpty { expected }
            | Self::RunNow { expected } => Some(expected),
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        if !matches!(target, ResourceTarget::Pool(_)) {
            return Err(Error::InvalidResourceCommand);
        }
        if let Some(e) = self.expected() {
            e.validate()?;
        }
        let valid = match self {
            Self::Create { config, .. } => config.validate().is_ok(),
            Self::Configure {
                expected, config, ..
            } => !expected.enabled && config != &expected.config && config.validate().is_ok(),
            Self::SetEnabled {
                expected,
                enabled,
                clock,
            } => {
                expected.enabled != *enabled
                    && (!*enabled || (!expected.members.is_empty() && clock.is_some()))
            }
            Self::Membership { expected, vm, join } => {
                !expected.enabled && expected.members.contains(vm) != *join
            }
            Self::DeleteEmpty { expected } => !expected.enabled && expected.members.is_empty(),
            Self::RunNow { expected } => expected.enabled && !expected.members.is_empty(),
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedules_reject_unsupported_resolution_and_ambiguous_weekdays() {
        for minute in [1, 14, 16, 59, 60] {
            assert!(SnapshotSchedule::Hourly { minute }.validate().is_err());
        }
        for days in [vec![], vec![0, 0], vec![7]] {
            assert!(
                SnapshotSchedule::Weekly {
                    hour: 2,
                    minute: 0,
                    days
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            SnapshotSchedule::Daily {
                hour: 24,
                minute: 0
            }
            .validate()
            .is_err()
        );
        for minute in [0, 15, 30, 45] {
            assert!(SnapshotSchedule::Hourly { minute }.validate().is_ok());
        }
    }
    #[test]
    fn enabled_empty_and_nonempty_delete_commands_fail_closed() {
        let pool = ResourceTarget::Pool(crate::PoolId::from(uuid::Uuid::new_v4()));
        let e = SnapshotPolicyState {
            id: SnapshotPolicyId::from(uuid::Uuid::new_v4()),
            config: SnapshotPolicyConfig {
                name: "Example".into(),
                description: "".into(),
                kind: ScheduledSnapshotKind::Disk,
                retained: 1,
                schedule: SnapshotSchedule::Hourly { minute: 0 },
            },
            enabled: false,
            members: vec![],
        };
        assert!(
            SnapshotPolicyOperation::RunNow {
                expected: e.clone()
            }
            .validate(pool)
            .is_err()
        );
        assert!(
            SnapshotPolicyOperation::SetEnabled {
                expected: e.clone(),
                enabled: true,
                clock: None
            }
            .validate(pool)
            .is_err()
        );
        let mut e = e;
        e.members.push(VmId::from(uuid::Uuid::new_v4()));
        assert!(
            SnapshotPolicyOperation::DeleteEmpty { expected: e }
                .validate(pool)
                .is_err()
        );
    }
}
