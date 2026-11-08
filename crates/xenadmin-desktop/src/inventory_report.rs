use super::*;
use serde::Serialize;
use std::io::Write;

const MAX_BYTES: usize = 8 * 1024 * 1024;
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
pub(super) fn encode(w: &Workspace) -> Result<Vec<u8>, String> {
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
                .and_then(|p| w.signature.iter().find(|s| s.0 == p));
            Coverage {
                pool: *pool,
                name: &source.name,
                current: source.current,
                metadata_complete: source.complete,
                generation: signature.map(|s| s.1),
                revision: signature.map(|s| s.2),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_keep_pool_scope_and_explicit_stale_incomplete_coverage() {
        let mut s = init();
        super::scene(&mut s);
        let w = edit(&mut s);
        w.unavailable = 1;
        let first = *w.sources.keys().next().unwrap();
        w.sources.get_mut(&first).unwrap().complete = false;
        let bytes = encode(w).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema"], "xenadmin-rs-inventory-report/v1");
        assert_eq!(value["connections_without_inventory"], 1);
        assert_eq!(value["resources"].as_array().unwrap().len(), w.rows.len());
        assert!(
            value["coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["current"] == false
                    && p["metadata_complete"] == false
                    && p["synthetic"] == true)
        );
        for row in value["resources"].as_array().unwrap() {
            assert!(row.get("pool").is_some());
            assert!(row.get("uuid").is_some());
            assert!(row.get("session").is_none());
        }
        let captured = bytes;
        w.rows.clear();
        assert!(
            !serde_json::from_slice::<serde_json::Value>(&captured).unwrap()["resources"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn report_limits_refuse_silent_truncation_or_invalid_filters() {
        let mut s = init();
        super::scene(&mut s);
        let w = edit(&mut s);
        let row = w.rows[0].clone();
        w.rows = vec![row.clone(); 10_001];
        assert!(encode(w).is_err());
        w.rows = vec![row];
        w.rows[0].description = "x".repeat(MAX_BYTES);
        assert!(encode(w).is_err());
        w.rows.clear();
        w.query.text = "x".repeat(513);
        assert!(encode(w).is_err());
    }
    #[test]
    fn native_report_export_captures_results_and_ignores_cancelled_or_stale_dialogs() {
        use std::os::unix::fs::PermissionsExt;
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = init();
            super::scene(&mut s);
            let mut rt = rune::Runtime::new(
                s,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                rt.frame(0.016);
            }
            let (x, y) = rt.find("search-export-results").unwrap();
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(0.016);
            assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Save);
            rt.resolve_file_dialog(None).unwrap();
            rt.frame(0.0);
            assert!(!rt.state.searches.as_ref().unwrap().files.busy);
            rt.update(files::export_results);
            let d = tempfile::tempdir().unwrap();
            let ignored = d.path().join("ignored.json");
            rt.update(|s, _| open(s));
            rt.resolve_file_dialog(Some(ignored.clone())).unwrap();
            rt.frame(0.0);
            assert!(!ignored.exists());
            rt.update(|s, tx| {
                super::scene(s);
                files::export_results(s, tx);
            });
            let expected = rt.state.searches.as_ref().unwrap().rows.len();
            rt.update(|s, _| edit(s).rows.clear());
            let path = d.path().join("report.json");
            rt.resolve_file_dialog(Some(path.clone())).unwrap();
            rt.frame(0.0);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while rt.state.searches.as_ref().unwrap().files.busy {
                assert!(std::time::Instant::now() < deadline);
                files::poll(&mut rt.state);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(value["resources"].as_array().unwrap().len(), expected);
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
