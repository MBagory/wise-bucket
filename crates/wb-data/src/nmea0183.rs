//! NMEA 0183 logs (`.nmea`): every sentence goes to the stream `sentences`, as written;
//! the sentences the `nmea` crate decodes also go to one stream per type (`GGA`, `RMC`…).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use chrono::{Days, NaiveDate, NaiveTime};
use datafusion::arrow::array::{
    ArrayRef, BooleanArray, RecordBatch, StringArray, TimestampNanosecondArray,
};
use datafusion::arrow::datatypes::{Field, Schema};
use nmea::ParseResult;
use serde_json::Value;
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::extract::{BATCH, Extraction};
use crate::rows::Rows;
use crate::sink::StreamSink;

/// The stream holding every sentence of a log.
pub const SENTENCES: &str = "sentences";

/// One sentence of the log.
struct Sentence<'a> {
    text: &'a str,
    /// `GP`, `GN`…; `None` for a proprietary sentence (`$P…`).
    talker: Option<&'a str>,
    /// `RMC`, `GGA`…, or the whole address of a proprietary sentence (`PGRME`).
    kind: &'a str,
    valid: bool,
    decoded: Option<ParseResult>,
}

/// Whether the checksum after `*` matches the sentence.
fn checksum_ok(text: &str) -> bool {
    let Some((body, sum)) = text[1..].rsplit_once('*') else {
        return false;
    };
    let computed = body.bytes().fold(0u8, |acc, b| acc ^ b);
    u8::from_str_radix(sum.trim(), 16) == Ok(computed)
}

/// The UTC time and date a decoded sentence gives.
fn time_and_date(r: &ParseResult) -> (Option<NaiveTime>, Option<NaiveDate>) {
    match r {
        ParseResult::RMC(d) => (d.fix_time, d.fix_date),
        ParseResult::GGA(d) => (d.fix_time, None),
        ParseResult::GLL(d) => (d.fix_time, None),
        ParseResult::GNS(d) => (d.fix_time, None),
        ParseResult::GBS(d) => (d.time, None),
        ParseResult::GST(d) => (d.time, None),
        ParseResult::ZDA(d) => (
            d.utc_time,
            match (d.year, d.month, d.day) {
                (Some(y), Some(m), Some(day)) => {
                    NaiveDate::from_ymd_opt(i32::from(y), u32::from(m), u32::from(day))
                }
                _ => None,
            },
        ),
        _ => (None, None),
    }
}

/// Leaves out the empty slots of arrays: a GSV sentence has four satellite slots, and
/// the crate gives `null` for those not used.
fn drop_empty_slots(v: &mut Value) {
    match v {
        Value::Array(items) => {
            items.retain(|i| !i.is_null());
            items.iter_mut().for_each(drop_empty_slots);
        }
        Value::Object(map) => map.values_mut().for_each(drop_empty_slots),
        _ => {}
    }
}

fn ns(date: NaiveDate, time: NaiveTime) -> Option<i64> {
    date.and_time(time).and_utc().timestamp_nanos_opt()
}

/// The time of each sentence: its own UTC time on the date of the latest `RMC` or `ZDA`
/// (before the first one, the first date); the day moves on when the time of day goes
/// back past midnight. A sentence without a time takes the time before it, the first
/// time for those before it. `None` when no sentence has a time.
fn times(sentences: &[Sentence<'_>]) -> (Vec<Option<i64>>, bool) {
    let stamps: Vec<_> = sentences
        .iter()
        .map(|s| s.decoded.as_ref().map_or((None, None), time_and_date))
        .collect();
    let first_date = stamps.iter().find_map(|(_, d)| *d);
    let mut date = first_date.unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid"));
    let mut previous: Option<NaiveTime> = None;
    let mut out = Vec::with_capacity(stamps.len());
    let mut last = None;
    for (time, d) in stamps {
        if let Some(d) = d {
            date = d;
        } else if let (Some(t), Some(p)) = (time, previous)
            && t < p
            && (p - t).num_hours() >= 12
        {
            date = date.checked_add_days(Days::new(1)).unwrap_or(date);
        }
        if let Some(t) = time {
            previous = Some(t);
            last = ns(date, t);
        }
        out.push(last);
    }
    // Before the first time: that first time.
    let first = out.iter().find_map(|t| *t);
    for t in &mut out {
        if t.is_some() {
            break;
        }
        *t = first;
    }
    (out, first_date.is_some())
}

/// The `sentences` stream, a batch at a time.
struct Raw {
    sink: StreamSink,
    t: Vec<Option<i64>>,
    talker: Vec<Option<String>>,
    kind: Vec<String>,
    text: Vec<String>,
    valid: Vec<bool>,
}

impl Raw {
    fn flush(&mut self) -> Result<()> {
        if self.t.is_empty() {
            return Ok(());
        }
        let arrays: Vec<(&str, ArrayRef)> = vec![
            (
                T,
                Arc::new(TimestampNanosecondArray::from(std::mem::take(&mut self.t))),
            ),
            (
                "talker",
                Arc::new(StringArray::from(std::mem::take(&mut self.talker))),
            ),
            (
                "type",
                Arc::new(StringArray::from(std::mem::take(&mut self.kind))),
            ),
            (
                "text",
                Arc::new(StringArray::from(std::mem::take(&mut self.text))),
            ),
            (
                "valid",
                Arc::new(BooleanArray::from(std::mem::take(&mut self.valid))),
            ),
        ];
        let fields: Vec<Field> = arrays
            .iter()
            .map(|(n, a)| match *n {
                T => Field::new(T, time_type(), true),
                _ => Field::new(*n, a.data_type().clone(), true),
            })
            .collect();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(fields)),
            arrays.into_iter().map(|(_, a)| a).collect(),
        )
        .map_err(|e| err(ErrorKind::Internal, format!("NMEA sentences: {e}")))?;
        self.sink.append(batch)
    }
}

pub(crate) async fn read(path: &Path, out: &Path) -> Result<Extraction> {
    let bytes = std::fs::read(path).map_err(crate::sink::io_err(path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let sentences: Vec<Sentence<'_>> = text
        .lines()
        .map(str::trim)
        .filter(|l| (l.starts_with('$') || l.starts_with('!')) && l.len() > 1)
        .map(|l| {
            let address = l[1..].split(',').next().unwrap_or("");
            let (talker, kind) = if address.starts_with('P') || address.len() < 3 {
                (None, address)
            } else {
                (Some(&address[..2]), &address[2..])
            };
            Sentence {
                text: l,
                talker,
                kind,
                valid: checksum_ok(l),
                decoded: nmea::parse_str(l).ok(),
            }
        })
        .collect();
    let (times, dated) = times(&sentences);

    let mut raw = Raw {
        sink: StreamSink::new(out, 0, SENTENCES, "nmea0183", "nmea0183"),
        t: Vec::new(),
        talker: Vec::new(),
        kind: Vec::new(),
        text: Vec::new(),
        valid: Vec::new(),
    };
    raw.sink.meta.decoded = true;
    let mut rows = Rows::new(out, 1);
    let mut not_decoded: BTreeMap<&str, u64> = BTreeMap::new();
    let mut bad = 0u64;
    for (s, t) in sentences.iter().zip(&times) {
        raw.t.push(*t);
        raw.talker.push(s.talker.map(str::to_string));
        raw.kind.push(s.kind.to_string());
        raw.text.push(s.text.to_string());
        raw.valid.push(s.valid);
        if raw.t.len() >= BATCH {
            raw.flush()?;
        }
        bad += u64::from(!s.valid);
        let decoded = match &s.decoded {
            Some(ParseResult::Unsupported(_)) | None => None,
            // Through text: the crate keeps some fields as f32, whose shortest decimal is
            // the value as written (14.7, not 14.699999809265137).
            Some(r) => serde_json::to_string(r)
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok()),
        };
        // The crate tags each sentence with its type: {"GGA": {…}}.
        match decoded {
            Some(Value::Object(tagged)) if tagged.len() == 1 => {
                let (name, fields) = tagged.into_iter().next().expect("one entry");
                let mut fields = match fields {
                    Value::Object(mut f) => {
                        f.values_mut().for_each(drop_empty_slots);
                        f
                    }
                    other => serde_json::Map::from_iter([("value".to_string(), other)]),
                };
                fields.insert("talker".into(), s.talker.map_or(Value::Null, |t| t.into()));
                rows.push(&name, "nmea0183", "nmea0183", t.unwrap_or(0), fields)?;
            }
            _ => *not_decoded.entry(s.kind).or_default() += 1,
        }
    }
    raw.flush()?;
    if !dated {
        raw.sink
            .note("no RMC or ZDA date: t is the time of day on 1970-01-01");
    }
    let mut extraction = rows.finish(out).await?;
    if let Some(meta) = raw.sink.finish(out)? {
        extraction.streams.insert(0, meta);
    }
    if bad > 0 {
        extraction.notes.push(format!(
            "{bad} sentence(s) with a bad checksum (stream {SENTENCES}, valid = false)"
        ));
    }
    if !not_decoded.is_empty() {
        let kinds: Vec<String> = not_decoded
            .iter()
            .map(|(k, n)| format!("{k} ({n})"))
            .collect();
        extraction.notes.push(format!(
            "not decoded, kept in stream {SENTENCES}: {}",
            kinds.join(", ")
        ));
    }
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Array, AsArray};
    use datafusion::arrow::datatypes::TimestampNanosecondType;

    fn stream(out: &Path, x: &Extraction, name: &str) -> RecordBatch {
        let meta = x.streams.iter().find(|s| s.name == name).unwrap();
        let file = std::fs::File::open(out.join(&meta.dataset)).unwrap();
        let reader =
            parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1 << 20).unwrap();
        let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
        datafusion::arrow::compute::concat_batches(&batches[0].schema(), &batches).unwrap()
    }

    fn t(b: &RecordBatch) -> Vec<i64> {
        let c = b
            .column_by_name(T)
            .unwrap()
            .as_primitive::<TimestampNanosecondType>();
        (0..c.len()).map(|i| c.value(i)).collect()
    }

    /// A sentence with its checksum.
    fn s(body: &str) -> String {
        format!("${body}*{:02X}", body.bytes().fold(0u8, |a, b| a ^ b))
    }

    #[tokio::test]
    async fn sentences_times_and_what_is_not_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("boat.nmea");
        let log = [
            "# a comment line".to_string(),
            s("GPGSV,1,1,01,01,78,221,47"),
            s("GPGGA,235959,4527.458,S,16709.165,E,1,05,2.1,14.7,M,1.1,M,,"),
            s("GPRMC,235959,A,4527.458,S,16709.165,E,000.0,094.5,060601,024.1,E"),
            s("PGRME,9.1,M,15.0,M,12.0,M"),
            // After midnight, before the next RMC: the next day.
            s("GPGGA,000001,4527.458,S,16709.165,E,1,05,2.1,14.8,M,1.1,M,,"),
            "$GPGGA,000002,4527.458,S,16709.165,E,1,05,2.1,14.9,M,1.1,M,,*00".to_string(),
        ]
        .join("\r\n");
        std::fs::write(&path, log).unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        let names: Vec<&str> = x.streams.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["sentences", "GSV", "GGA", "RMC"]);

        let day = 991_785_600_000_000_000; // 2001-06-06T00:00:00Z
        let raw = stream(&out, &x, SENTENCES);
        assert_eq!(raw.num_rows(), 6);
        let second = 1_000_000_000;
        // The GSV before the first time takes that first time.
        assert_eq!(
            t(&raw),
            [
                day + 86_399 * second,
                day + 86_399 * second,
                day + 86_399 * second,
                day + 86_399 * second,
                day + 86_401 * second,
                day + 86_401 * second
            ]
        );
        let kinds = raw.column_by_name("type").unwrap().as_string::<i32>();
        assert_eq!((kinds.value(0), kinds.value(3)), ("GSV", "PGRME"));
        assert!(raw.column_by_name("talker").unwrap().is_null(3));
        let valid = raw.column_by_name("valid").unwrap().as_boolean();
        assert_eq!((valid.value(4), valid.value(5)), (true, false));
        // The bad checksum is not decoded.
        assert_eq!(stream(&out, &x, "GGA").num_rows(), 2);
        assert_eq!(
            x.notes,
            [
                "1 sentence(s) with a bad checksum (stream sentences, valid = false)",
                "not decoded, kept in stream sentences: GGA (1), PGRME (1)"
            ]
        );
    }

    #[tokio::test]
    async fn without_a_date_the_day_is_1970() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gga.nmea");
        std::fs::write(
            &path,
            s("GPGGA,000010,4527.458,S,16709.165,E,1,05,2.1,14.7,M,1.1,M,,"),
        )
        .unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("data")).unwrap();
        let x = read(&path, &out).await.unwrap();
        assert_eq!(t(&stream(&out, &x, "GGA")), [10_000_000_000]);
        let raw = x.streams.iter().find(|s| s.name == SENTENCES).unwrap();
        assert_eq!(
            raw.notes,
            ["no RMC or ZDA date: t is the time of day on 1970-01-01"]
        );
    }
}
