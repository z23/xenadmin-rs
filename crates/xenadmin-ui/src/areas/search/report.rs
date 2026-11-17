//! The inventory report: the Rune `inventory_report.rs`.
//!
//! The report is the current search results with their coverage: which
//! pools were searched, whether each was current and its metadata complete.
//! It refuses rather than truncates.

use std::collections::BTreeMap;
use std::io::Write;

use serde::Serialize;
use xenadmin_core::{InventorySearch, PoolId, PowerState};

use super::query::Sheet;

pub(super) const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize)]
struct Coverage<'a> {
    pool: PoolId,
    name: &'a str,
    current: bool,
    metadata_complete: bool,
    generation: Option<u64>,
    revision: Option<u64>,
    synthetic: bool,
}
#[derive(Serialize)]
struct Row<'a> {
    pool: PoolId,
    uuid: uuid::Uuid,
    kind: &'a str,
    name: &'a str,
    description: &'a str,
    host: &'a str,
    power: Option<&'a PowerState>,
    tags: Option<&'a [String]>,
    folder: Option<&'a str>,
    metadata_known: bool,
    custom_fields: &'a BTreeMap<String, String>,
}
#[derive(Serialize)]
struct Report<'a> {
    schema: &'static str,
    captured_at_ms: u64,
    filters: &'a InventorySearch,
    coverage: Vec<Coverage<'a>>,
    connections_without_inventory: usize,
    resources: Vec<Row<'a>>,
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_BYTES - self.0.len() {
            return Err(std::io::Error::other("Report exceeds 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn encode(w: &Sheet) -> Result<Vec<u8>, String> {
    w.query.validate()?;
    if w.rows.len() > 10_000 {
        return Err("Filter the results to at most 10,000 resources before exporting".into());
    }
    if w.error.is_some() {
        return Err("Resolve the search error before exporting results".into());
    }
    let coverage = w
        .sources
        .iter()
        .map(|(pool, source)| {
            let signature = source
                .profile
                .and_then(|p| w.signature.iter().find(|s| s.profile == p));
            Coverage {
                pool: *pool,
                name: &source.name,
                current: source.current,
                metadata_complete: source.complete,
                generation: signature.and_then(|s| s.generation),
                revision: signature.and_then(|s| s.revision),
                synthetic: source.profile.is_none(),
            }
        })
        .collect();
    let resources = w
        .rows
        .iter()
        .map(|r| Row {
            pool: r.pool,
            uuid: r.object.uuid(),
            kind: r.kind.label(),
            name: &r.name,
            description: &r.description,
            host: &r.host,
            power: r.power.as_ref(),
            tags: r.tags.as_deref(),
            folder: r.folder.as_deref(),
            metadata_known: r.metadata_known,
            custom_fields: &r.custom_fields,
        })
        .collect();
    let report = Report {
        schema: "xenadmin-rs-inventory-report/v1",
        captured_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        filters: &w.query,
        coverage,
        connections_without_inventory: w.unavailable,
        resources,
    };
    let mut bytes = Bounded(Vec::new());
    serde_json::to_writer_pretty(&mut bytes, &report)
        .map_err(|_| "Cannot encode this report within 8 MiB; narrow the filters".to_owned())?;
    Ok(bytes.0)
}
