//! Checks run while a stream is written, to point the agent at what to look at first:
//! gaps in periodic streams, periodic streams that stop early, `t` going backwards,
//! `t_sensor` drifting from `t`, and float columns that are constant or empty.
//! Thresholds are heuristics ([ADR 0032](../../../docs/decisions/0032-extraction-checks-in-notes.md)).

use chrono::DateTime;
use datafusion::arrow::array::{Array, ArrayRef, AsArray, RecordBatch};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Float64Type, Int64Type};
use serde::{Deserialize, Serialize};

use crate::columns::{T, T_SENSOR};
use crate::sink::StreamMeta;

const SEC: i64 = 1_000_000_000;
/// A gap is an interval longer than this many median intervals…
const GAP_FACTOR: i64 = 5;
/// …and longer than this.
const MIN_GAP: i64 = SEC;
/// A stream is periodic when this share of its intervals is within ±50% of the median.
const PERIODIC_SHARE: f64 = 0.8;
/// Fewest intervals to call a stream periodic.
const MIN_INTERVALS: usize = 5;
/// A periodic stream stops early when it ends more than this share of the recording
/// (and more than a gap) before the recording does.
const EARLY_SHARE: f64 = 0.05;
/// `t - t_sensor` varying by more than this is a drift or a jump.
const MAX_OFFSET_RANGE: i64 = SEC;
/// Intervals kept to find the median, at most.
const SAMPLE: u64 = 1_000_000;
/// Column names quoted in a note, at most.
const NAMES: usize = 10;
/// Times before 2000 count from boot (PX4 ULog).
const Y2000: i64 = 946_684_800 * SEC;

/// What the checks found in one stream, for the recording's summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checks {
    /// Median interval, when the stream is periodic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_ns: Option<i64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gaps: u64,
    /// Largest gap: (start, duration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gap: Option<(i64, i64)>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub backwards: u64,
    /// Every float column is constant or without values.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flat: bool,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Checks {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Values of one float column: min, max and how many there are (nulls and NaN left out).
struct Floats {
    name: String,
    min: f64,
    max: f64,
    count: u64,
}

/// Accumulates one stream's batches.
pub struct Scan {
    stride: u64,
    index: u64,
    last: Option<i64>,
    sample: Vec<i64>,
    /// Intervals of at least [`MIN_GAP`]: (start, duration).
    long: Vec<(i64, i64)>,
    backwards: u64,
    first_backwards: Option<i64>,
    offset: Option<(i64, i64)>,
    floats: Vec<Floats>,
}

impl Scan {
    pub fn new(rows: u64) -> Self {
        Self {
            stride: rows.div_ceil(SAMPLE).max(1),
            index: 0,
            last: None,
            sample: Vec::new(),
            long: Vec::new(),
            backwards: 0,
            first_backwards: None,
            offset: None,
            floats: Vec::new(),
        }
    }

    pub fn observe(&mut self, batch: &RecordBatch) {
        let int = |name: &str| {
            batch
                .column_by_name(name)
                .and_then(|c| cast(c, &DataType::Int64).ok())
        };
        let t = int(T);
        if let Some(t) = &t {
            for v in t.as_primitive::<Int64Type>().iter().flatten() {
                if let Some(last) = self.last {
                    let d = v - last;
                    if d < 0 {
                        self.backwards += 1;
                        self.first_backwards.get_or_insert(last);
                    }
                    if self.index.is_multiple_of(self.stride) {
                        self.sample.push(d);
                    }
                    if d >= MIN_GAP {
                        self.long.push((last, d));
                    }
                    self.index += 1;
                }
                self.last = Some(v);
            }
        }
        if let (Some(t), Some(s)) = (&t, int(T_SENSOR)) {
            let (t, s) = (t.as_primitive::<Int64Type>(), s.as_primitive::<Int64Type>());
            for i in 0..t.len() {
                // An unset header stamp (0) says nothing about the clocks.
                if t.is_null(i) || s.is_null(i) || s.value(i) <= 0 {
                    continue;
                }
                let o = t.value(i) - s.value(i);
                self.offset = Some(self.offset.map_or((o, o), |(a, b)| (a.min(o), b.max(o))));
            }
        }
        for (i, field) in batch.schema().fields().iter().enumerate() {
            self.floats_in(field.name(), batch.column(i));
        }
    }

    /// Float columns, nested fields included (`linear_acceleration.x`).
    fn floats_in(&mut self, name: &str, column: &ArrayRef) {
        match column.data_type() {
            DataType::Struct(fields) => {
                let s = column.as_struct();
                for (field, child) in fields.iter().zip(s.columns()) {
                    self.floats_in(&format!("{name}.{}", field.name()), child);
                }
            }
            DataType::Float16 | DataType::Float32 | DataType::Float64 => {
                let Ok(values) = cast(column, &DataType::Float64) else {
                    return;
                };
                let f = match self.floats.iter().position(|f| f.name == name) {
                    Some(p) => &mut self.floats[p],
                    None => {
                        self.floats.push(Floats {
                            name: name.to_string(),
                            min: f64::INFINITY,
                            max: f64::NEG_INFINITY,
                            count: 0,
                        });
                        self.floats.last_mut().expect("just pushed")
                    }
                };
                for v in values.as_primitive::<Float64Type>().iter().flatten() {
                    if !v.is_nan() {
                        f.min = f.min.min(v);
                        f.max = f.max.max(v);
                        f.count += 1;
                    }
                }
            }
            _ => {}
        }
    }

    /// The stream's checks and its notes.
    pub fn finish(mut self, rows: u64) -> (Checks, Vec<String>) {
        let mut checks = Checks::default();
        let mut notes = Vec::new();

        // Gaps and backward steps mean something in periodic streams only: event
        // streams, records and tables are not time-sorted or regular by nature.
        if let Some(period) = periodic(&mut self.sample) {
            checks.period_ns = Some(period);
            let gaps: Vec<_> = self
                .long
                .iter()
                .filter(|(_, d)| *d > GAP_FACTOR * period)
                .collect();
            checks.gaps = gaps.len() as u64;
            checks.max_gap = gaps.iter().max_by_key(|(_, d)| *d).map(|g| **g);
            if let Some((at, d)) = checks.max_gap {
                notes.push(format!(
                    "{} gap(s) longer than {GAP_FACTOR}× the median interval ({}), largest {} at {}",
                    checks.gaps,
                    duration(period),
                    duration(d),
                    time(at)
                ));
            }
            if self.backwards > 0 {
                checks.backwards = self.backwards;
                notes.push(format!(
                    "t goes backwards {} time(s), first after {}",
                    self.backwards,
                    time(self.first_backwards.unwrap_or_default())
                ));
            }
        }
        if let Some((a, b)) = self.offset
            && b - a > MAX_OFFSET_RANGE
        {
            notes.push(format!(
                "t - t_sensor varies from {} to {}",
                signed(a),
                signed(b)
            ));
        }
        // Many fields are constant by design; a stream whose every float column is
        // constant or empty looks like a sensor that does not work.
        let flat = |f: &Floats| f.count == 0 || f.min == f.max;
        if rows > MIN_INTERVALS as u64 && !self.floats.is_empty() && self.floats.iter().all(flat) {
            checks.flat = true;
            let values: Vec<String> = self
                .floats
                .iter()
                .map(|f| match f.count {
                    0 => format!("{} = null", f.name),
                    _ => format!("{} = {}", f.name, f.min),
                })
                .collect();
            notes.push(format!(
                "every float column is constant or without values: {}",
                names(&values)
            ));
        }
        (checks, notes)
    }
}

/// The median interval, when most intervals are close to it.
fn periodic(sample: &mut [i64]) -> Option<i64> {
    if sample.len() < MIN_INTERVALS {
        return None;
    }
    let mid = sample.len() / 2;
    let median = *sample.select_nth_unstable(mid).1;
    if median <= 0 {
        return None;
    }
    let close = sample
        .iter()
        .filter(|d| (**d - median).abs() * 2 <= median)
        .count();
    (close as f64 >= PERIODIC_SHARE * sample.len() as f64).then_some(median)
}

/// The recording's notes from its streams' checks.
pub fn summary(streams: &[StreamMeta]) -> Vec<String> {
    let mut notes = Vec::new();
    // The span of the periodic streams: records (parameters, texts) may be stamped
    // long after the last sample.
    let periodic = || streams.iter().filter(|s| s.checks.period_ns.is_some());
    let span = periodic()
        .filter_map(|s| s.start_ns)
        .min()
        .zip(periodic().filter_map(|s| s.end_ns).max());

    let gapped: Vec<_> = streams.iter().filter(|s| s.checks.gaps > 0).collect();
    if let Some((s, (at, d))) = gapped
        .iter()
        .filter_map(|s| s.checks.max_gap.map(|g| (s, g)))
        .max_by_key(|(_, (_, d))| *d)
    {
        notes.push(format!(
            "{} gap(s) in {} periodic stream(s), largest {} in {} at {}",
            gapped.iter().map(|s| s.checks.gaps).sum::<u64>(),
            gapped.len(),
            duration(d),
            s.name,
            time(at)
        ));
    }

    let mut early: Vec<(&str, i64)> = periodic()
        .filter_map(|s| {
            let (start, end) = span?;
            let early_min = ((end - start) as f64 * EARLY_SHARE) as i64;
            let (period, last) = (s.checks.period_ns?, s.end_ns?);
            let before = end - last;
            (before > early_min && before > MIN_GAP.max(GAP_FACTOR * period))
                .then_some((s.name.as_str(), before))
        })
        .collect();
    early.sort_by_key(|(_, before)| std::cmp::Reverse(*before));
    if let Some((name, before)) = early.first() {
        notes.push(format!(
            "{} periodic stream(s) stop before the end, earliest {} ({} before)",
            early.len(),
            name,
            duration(*before)
        ));
    }

    let backwards: Vec<String> = streams
        .iter()
        .filter(|s| s.checks.backwards > 0)
        .map(|s| s.name.clone())
        .collect();
    if !backwards.is_empty() {
        notes.push(format!(
            "t goes backwards in {} stream(s): {}",
            backwards.len(),
            names(&backwards)
        ));
    }

    let flat: Vec<String> = streams
        .iter()
        .filter(|s| s.checks.flat)
        .map(|s| s.name.clone())
        .collect();
    if !flat.is_empty() {
        notes.push(format!(
            "every float column constant or without values in {} stream(s): {}",
            flat.len(),
            names(&flat)
        ));
    }
    notes
}

fn names(items: &[String]) -> String {
    let shown = items[..items.len().min(NAMES)].join(", ");
    match items.len().saturating_sub(NAMES) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

fn duration(ns: i64) -> String {
    let s = ns as f64 / SEC as f64;
    match s {
        s if s < 1.0 => format!("{} ms", trim(s * 1e3)),
        s if s < 120.0 => format!("{} s", trim(s)),
        s if s < 7200.0 => format!("{} min", trim(s / 60.0)),
        s => format!("{} h", trim(s / 3600.0)),
    }
}

fn signed(ns: i64) -> String {
    if ns < 0 {
        format!("-{}", duration(-ns))
    } else {
        duration(ns)
    }
}

/// Up to 3 significant decimals, without trailing zeros.
fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A UTC time, or seconds since boot before 2000.
fn time(ns: i64) -> String {
    if ns < Y2000 {
        return format!("{} s", trim(ns as f64 / SEC as f64));
    }
    DateTime::from_timestamp_nanos(ns)
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string()
        .trim_end_matches(".000")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::array::{Float64Array, TimestampNanosecondArray};
    use datafusion::arrow::datatypes::{Field, Schema, TimeUnit};

    use super::*;

    fn batch(t: &[i64], x: &[f64]) -> RecordBatch {
        let schema = Schema::new(vec![
            Field::new(T, DataType::Timestamp(TimeUnit::Nanosecond, None), false),
            Field::new("x", DataType::Float64, true),
        ]);
        RecordBatch::try_new(
            Arc::new(schema),
            vec![
                Arc::new(TimestampNanosecondArray::from(t.to_vec())),
                Arc::new(Float64Array::from(x.to_vec())),
            ],
        )
        .unwrap()
    }

    fn scan(t: &[i64], x: &[f64]) -> (Checks, Vec<String>) {
        let mut s = Scan::new(t.len() as u64);
        // Two batches, so intervals across batches count.
        let mid = t.len() / 2;
        s.observe(&batch(&t[..mid], &x[..mid]));
        s.observe(&batch(&t[mid..], &x[mid..]));
        s.finish(t.len() as u64)
    }

    #[test]
    fn a_gap_in_a_periodic_stream() {
        // The NMEA sample: RMC every 2 s, then 16 s.
        let t: Vec<i64> = [0, 2, 4, 6, 8, 10, 26].iter().map(|s| s * SEC).collect();
        let (c, notes) = scan(&t, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(c.period_ns, Some(2 * SEC));
        assert_eq!((c.gaps, c.max_gap), (1, Some((10 * SEC, 16 * SEC))));
        assert_eq!(
            notes,
            ["1 gap(s) longer than 5× the median interval (2 s), largest 16 s at 10 s"]
        );
    }

    #[test]
    fn irregular_and_shared_times_are_not_periodic() {
        let t: Vec<i64> = [0, 1, 7, 8, 30, 31, 90].iter().map(|s| s * SEC).collect();
        assert_eq!(scan(&t, &[0.5; 7]).0.period_ns, None);
        let t = [5 * SEC; 8];
        assert_eq!(scan(&t, &[0.5; 8]).0.period_ns, None);
    }

    #[test]
    fn backwards_time_and_flat_columns() {
        let t: Vec<i64> = [0, 1, 2, 3, 4, 5, 6, 5, 7, 8, 9, 10, 11]
            .iter()
            .map(|s| s * SEC)
            .collect();
        let (c, notes) = scan(&t, &[0.0; 13]);
        assert_eq!((c.backwards, c.flat), (1, true));
        assert_eq!(
            notes,
            [
                "t goes backwards 1 time(s), first after 6 s",
                "every float column is constant or without values: x = 0"
            ]
        );
        let (c, notes) = scan(&t, &[f64::NAN; 13]);
        assert!(c.flat);
        assert_eq!(
            notes[1],
            "every float column is constant or without values: x = null"
        );
        // Not periodic: no backward step reported.
        let t: Vec<i64> = [0, 9, 2, 30, 31, 70].iter().map(|s| s * SEC).collect();
        assert_eq!(scan(&t, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).0.backwards, 0);
    }

    #[test]
    fn times_read_as_dates_or_seconds() {
        assert_eq!(time(112_500_000_000), "112.5 s");
        assert_eq!(time(991_867_998 * SEC), "2001-06-06 22:53:18");
        assert_eq!(duration(16 * SEC), "16 s");
        assert_eq!(duration(SEC / 50), "20 ms");
        assert_eq!(duration(600 * SEC), "10 min");
    }
}
