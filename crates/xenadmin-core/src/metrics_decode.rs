use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::records::DataSourceRecord;
use xenadmin_xapi::{Reads, kind};
const MAX_SOURCES: usize = 512;
const MAX_COLUMNS: usize = 2048;
const MAX_ROWS: usize = 2048;
pub(super) const MAX_SAMPLES: usize = 2048;
pub(super) struct Batch {
    pub step: u64,
    pub end: u64,
    pub series: Vec<MetricSeries>,
}
fn integer(value: &Value) -> Result<u64, Error> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .ok_or(Error::Protocol("invalid metrics timestamp or count"))
}
fn text<'a>(value: impl Reads<'a, kind::Text>, limit: usize) -> Result<String, Error> {
    let value = value.raw();
    let value = value
        .as_str()
        .filter(|v| v.len() <= limit && !v.chars().any(|c| c.is_control() && !c.is_whitespace()))
        .ok_or(Error::Protocol("invalid metric source metadata"))?;
    Ok(value.into())
}
pub(super) fn sources(value: &Value) -> Result<Vec<MetricSource>, Error> {
    let values = value
        .as_array()
        .filter(|values| values.len() <= MAX_SOURCES)
        .ok_or(Error::Protocol(
            "metric source list exceeds the limit or is malformed",
        ))?;
    let mut names = BTreeSet::new();
    let mut sources = Vec::new();
    for value in values {
        let name = text(DataSourceRecord::new(value).name_label(), 128)?;
        // One source this client cannot name must not take every chart on the host with
        // it: such a source, or a repeat of one already listed, is left out.
        if !valid_source(&name) || !names.insert(name.clone()) {
            continue;
        }
        sources.push(MetricSource {
            category: MetricCategory::for_name(&name),
            name,
            description: text(DataSourceRecord::new(value).name_description(), 2048)?,
            units: text(DataSourceRecord::new(value).units(), 128)?,
            enabled: DataSourceRecord::new(value)
                .enabled()
                .get()
                .ok_or(Error::Protocol("metric recording state unavailable"))?,
        });
    }
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sources)
}
pub(super) fn select(sources: &mut Vec<MetricSource>, requested: &[String]) -> Vec<String> {
    if requested.is_empty() {
        let mut selected = Vec::new();
        for category in [
            MetricCategory::Cpu,
            MetricCategory::Memory,
            MetricCategory::Disk,
            MetricCategory::Network,
        ] {
            selected.extend(
                sources
                    .iter()
                    .filter(|source| source.enabled && source.category == category)
                    .take(8)
                    .map(|source| source.name.clone()),
            );
        }
        selected
    } else {
        for name in requested {
            if !sources.iter().any(|source| source.name == *name) {
                sources.push(MetricSource {
                    name: name.clone(),
                    description: "Not reported by this host".into(),
                    units: "unknown".into(),
                    enabled: false,
                    category: MetricCategory::for_name(name),
                });
            }
        }
        requested.to_vec()
    }
}
fn sample(value: &Value) -> Result<Option<f64>, Error> {
    if value.is_null() {
        return Ok(None);
    }
    let number = if let Some(value) = value.as_f64() {
        value
    } else if let Some(value) = value.as_str() {
        if matches!(
            value.to_ascii_lowercase().as_str(),
            "nan" | "-nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity"
        ) {
            return Ok(None);
        }
        value
            .parse::<f64>()
            .map_err(|_| Error::Protocol("invalid metric sample"))?
    } else {
        return Err(Error::Protocol("invalid metric sample"));
    };
    Ok(number.is_finite().then_some(number))
}
pub(super) fn decode(
    bytes: &[u8],
    target: MetricTarget,
    sources: &[MetricSource],
    selected: &[String],
) -> Result<Batch, Error> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| Error::Protocol("invalid metrics JSON"))?;
    let meta = &value["meta"];
    let start = integer(&meta["start"])?;
    let end = integer(&meta["end"])?;
    let step = integer(&meta["step"])?;
    // XCP-ng reports `start` as the requested start rounded up to the next sample, yet it
    // also returns the sample at the requested instant, and when nothing newer exists it
    // reports a start one step past its end with that one row or none. Both are ordinary
    // answers to a refresh, so the bounds allow one step of slack; anything further is
    // still refused.
    if step == 0 || step > 86_400 * 366 || start > end.saturating_add(step) {
        return Err(Error::Protocol("invalid metrics sampling bounds"));
    }
    let earliest = start.saturating_sub(step);
    let legends = meta["legend"]
        .as_array()
        .filter(|values| values.len() <= MAX_COLUMNS)
        .ok_or(Error::Protocol("invalid or oversized metric legend"))?;
    let rows = value["data"]
        .as_array()
        .filter(|values| values.len() <= MAX_ROWS)
        .ok_or(Error::Protocol("invalid or oversized metric rows"))?;
    if integer(&meta["columns"])? != legends.len() as u64
        || integer(&meta["rows"])? != rows.len() as u64
    {
        return Err(Error::Protocol("metrics dimensions do not match"));
    }
    let prefix = format!(
        "AVERAGE:{}:{}:",
        target.class().to_ascii_lowercase(),
        target.uuid()
    );
    let mut columns = BTreeMap::new();
    for (column, legend) in legends.iter().enumerate() {
        let legend = legend
            .as_str()
            .filter(|s| s.len() <= 256)
            .ok_or(Error::Protocol("invalid metric legend"))?;
        // A column for a source this client does not list is ignored, as its source is; a
        // second column for the same source is ambiguous and refuses the answer.
        if let Some(name) = legend.strip_prefix(&prefix)
            && valid_source(name)
            && columns.insert(name, column).is_some()
        {
            return Err(Error::Protocol("duplicate metric column"));
        }
    }
    let mut ordered = BTreeMap::new();
    for row in rows {
        let timestamp = integer(&row["t"])?;
        let values = row["values"]
            .as_array()
            .filter(|values| values.len() == legends.len())
            .ok_or(Error::Protocol(
                "metrics row width does not match its legend",
            ))?;
        if timestamp < earliest || timestamp > end || ordered.insert(timestamp, values).is_some() {
            return Err(Error::Protocol("invalid or duplicate metric timestamp"));
        }
    }
    let mut series = Vec::new();
    for name in selected {
        let enabled = sources
            .iter()
            .any(|source| source.name == *name && source.enabled);
        let column = columns.get(name.as_str()).filter(|_| enabled);
        let samples = ordered
            .iter()
            .map(|(&timestamp, values)| {
                Ok(MetricSample {
                    timestamp,
                    value: column.map(|&i| sample(&values[i])).transpose()?.flatten(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        series.push(MetricSeries {
            source: name.clone(),
            samples,
        });
    }
    Ok(Batch { step, end, series })
}
pub(super) fn merge(
    snapshot: &mut MetricsSnapshot,
    batch: Batch,
    window: MetricsWindow,
    reset: bool,
) {
    let cutoff = batch.end.saturating_sub(window.duration_seconds());
    if reset
        || snapshot
            .sample_step_seconds
            .is_some_and(|step| step != batch.step)
    {
        snapshot.series.clear();
    }
    let mut series = Vec::new();
    for incoming in batch.series {
        let mut points = BTreeMap::new();
        if let Some(previous) = snapshot
            .series
            .iter()
            .find(|series| series.source == incoming.source)
        {
            for point in &previous.samples {
                points.insert(point.timestamp, point.value);
            }
        }
        for point in incoming.samples {
            points.insert(point.timestamp, point.value);
        }
        points.retain(|timestamp, _| *timestamp >= cutoff && *timestamp <= batch.end);
        while points.len() > MAX_SAMPLES {
            points.pop_first();
        }
        series.push(MetricSeries {
            source: incoming.source,
            samples: points
                .into_iter()
                .map(|(timestamp, value)| MetricSample { timestamp, value })
                .collect(),
        });
    }
    snapshot.range_start = series
        .iter()
        .filter_map(|series| series.samples.first().map(|point| point.timestamp))
        .min();
    snapshot.range_end = series
        .iter()
        .filter_map(|series| series.samples.last().map(|point| point.timestamp))
        .max();
    snapshot.sample_step_seconds = Some(batch.step);
    snapshot.series = series;
}
