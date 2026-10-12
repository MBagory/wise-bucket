//! Sends the rows of a query to a running Rerun viewer, without a size cap: they go to
//! the viewer, not to the agent's context. Each timestamp column (`t`, `t_sensor`…)
//! becomes a timeline; each numeric or boolean column a scalar series.

use std::collections::HashMap;

use datafusion::arrow::array::{Array, AsArray, RecordBatch};
use datafusion::arrow::compute::cast;
use datafusion::arrow::datatypes::{DataType, Float64Type, TimestampNanosecondType};
use datafusion::prelude::DataFrame;
use futures::StreamExt;
use rerun::{RecordingStream, RecordingStreamBuilder, Scalars, TimeColumn, TimelineName};
use serde::Serialize;
use wb_core::error::{ErrorKind, Result, err};

use crate::columns::{T, time_type};
use crate::sql::{self, query_err};
use crate::store::Store;

/// Address of a viewer started with `rerun`.
pub const DEFAULT_URL: &str = "rerun+http://127.0.0.1:9876/proxy";
/// Rerun application and recording id: successive sends add to one viewer recording.
// ponytail: sending the same query twice duplicates its rows; per-call recording ids if that bites.
const RECORDING: &str = "wisebucket";
/// Times before 2000-01-01 count from boot (PX4 ULog, MAVLink): shown as durations.
const YEAR_2000_NS: i64 = 946_684_800_000_000_000;

/// What was sent.
#[derive(Debug, Clone, Serialize)]
pub struct Sent {
    pub rows: usize,
    /// Entity paths of the scalar series.
    pub entities: Vec<String>,
    pub timelines: Vec<String>,
    /// Columns that are neither times nor numbers.
    pub skipped: Vec<String>,
    pub recording: &'static str,
    pub url: String,
}

/// Runs one read-only query and sends its rows to the viewer at `url`, under
/// `<entity>/<column>` entity paths.
pub async fn send(store: &Store, query: &str, entity: Option<&str>, url: &str) -> Result<Sent> {
    let unreachable = |e: &dyn std::fmt::Display| {
        err(
            ErrorKind::ViewerUnreachable,
            format!("Rerun viewer at {url}: {e}"),
        )
    };
    let rec = RecordingStreamBuilder::new(RECORDING)
        .recording_id(RECORDING)
        .connect_grpc_opts(url)
        .map_err(|e| unreachable(&e))?;
    let df = sql::dataframe(store, query).await?;
    let mut sent = tokio::time::timeout(sql::TIMEOUT, send_rows(&rec, df, entity))
        .await
        .map_err(|_| {
            query_err(format!(
                "the query ran for more than {} s and was stopped",
                sql::TIMEOUT.as_secs()
            ))
        })??;
    tokio::task::spawn_blocking(move || rec.flush_with_timeout(sql::TIMEOUT))
        .await
        .map_err(|e| err(ErrorKind::Internal, e.to_string()))?
        .map_err(|e| unreachable(&e))?;
    sent.url = url.to_string();
    Ok(sent)
}

async fn send_rows(rec: &RecordingStream, df: DataFrame, entity: Option<&str>) -> Result<Sent> {
    let mut sent = Sent {
        rows: 0,
        entities: Vec::new(),
        timelines: Vec::new(),
        skipped: Vec::new(),
        recording: RECORDING,
        url: String::new(),
    };
    let mut values = Vec::new();
    for (i, f) in df.schema().fields().iter().enumerate() {
        match f.data_type() {
            DataType::Timestamp(..) => sent.timelines.push(f.name().clone()),
            t if t.is_numeric() || *t == DataType::Boolean => {
                values.push(i);
                let name = f.name();
                sent.entities
                    .push(entity.map_or(name.clone(), |p| format!("{p}/{name}")));
            }
            _ => sent.skipped.push(f.name().clone()),
        }
    }
    let Some(t) = df.schema().fields().iter().position(|f| f.name() == T) else {
        return Err(query_err(
            "the query must return a timestamp column `t`: it becomes the time axis in Rerun",
        ));
    };
    if !sent.timelines.iter().any(|n| n == T) {
        return Err(query_err("column `t` must be a timestamp"));
    }
    let times: Vec<usize> = df
        .schema()
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f.data_type(), DataType::Timestamp(..)))
        .map(|(i, _)| i)
        .collect();
    // Whether each timeline counts from boot, decided on its first value.
    let mut durations: HashMap<usize, bool> = HashMap::new();
    let mut stream = df.execute_stream().await.map_err(query_err)?;
    while let Some(batch) = stream.next().await {
        let batch = batch.map_err(query_err)?;
        sent.rows += batch.num_rows() - batch.column(t).null_count();
        send_batch(
            rec,
            &batch,
            t,
            &times,
            &values,
            &sent.entities,
            &mut durations,
        )?;
    }
    Ok(sent)
}

fn send_batch(
    rec: &RecordingStream,
    batch: &RecordBatch,
    t: usize,
    times: &[usize],
    values: &[usize],
    entities: &[String],
    durations: &mut HashMap<usize, bool>,
) -> Result<()> {
    let ns: Vec<_> = times
        .iter()
        .map(|&i| {
            let a = cast(batch.column(i), &time_type()).map_err(query_err)?;
            Ok((i, a.as_primitive::<TimestampNanosecondType>().clone()))
        })
        .collect::<Result<_>>()?;
    let t_ns = &ns.iter().find(|(i, _)| *i == t).expect("t is a timeline").1;
    for (&v, path) in values.iter().zip(entities) {
        let v = cast(batch.column(v), &DataType::Float64).map_err(query_err)?;
        let v = v.as_primitive::<Float64Type>();
        let rows: Vec<usize> = (0..batch.num_rows())
            .filter(|&r| t_ns.is_valid(r) && v.is_valid(r))
            .collect();
        if rows.is_empty() {
            continue;
        }
        // A timeline with a null among these rows is left out of this chunk.
        let timelines = ns
            .iter()
            .filter(|(_, a)| rows.iter().all(|&r| a.is_valid(r)))
            .map(|(i, a)| {
                let name =
                    TimelineName::try_new(batch.schema().field(*i).name()).map_err(query_err)?;
                let values = rows.iter().map(|&r| a.value(r));
                Ok(
                    if *durations
                        .entry(*i)
                        .or_insert(a.value(rows[0]) < YEAR_2000_NS)
                    {
                        TimeColumn::new_duration_nanos(name, values)
                    } else {
                        TimeColumn::new_timestamp_nanos_since_epoch(name, values)
                    },
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let scalars = Scalars::new(rows.iter().map(|&r| v.value(r)))
            .columns_of_unit_batches()
            .map_err(query_err)?;
        rec.send_columns(path.as_str(), timelines, scalars)
            .map_err(query_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::arrow::array::{
        Float64Array, Int32Array, StringArray, TimestampNanosecondArray,
    };
    use datafusion::arrow::datatypes::{Field, Schema};
    use datafusion::prelude::SessionContext;

    use super::*;

    #[tokio::test]
    async fn sends_numbers_on_their_timelines() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("t", time_type(), true),
            Field::new("t_sensor", time_type(), true),
            Field::new("z", DataType::Float64, true),
            Field::new("n", DataType::Int32, true),
            Field::new("recording", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(TimestampNanosecondArray::from(vec![Some(1), Some(2), None])),
                Arc::new(TimestampNanosecondArray::from(vec![Some(1), None, Some(3)])),
                Arc::new(Float64Array::from(vec![Some(1.0), None, Some(3.0)])),
                Arc::new(Int32Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["a", "a", "a"])),
            ],
        )
        .unwrap();
        let df = SessionContext::new().read_batch(batch).unwrap();
        let (rec, storage) = RecordingStreamBuilder::new("test").memory().unwrap();
        let sent = send_rows(&rec, df, Some("imu")).await.unwrap();
        assert_eq!(sent.rows, 2);
        assert_eq!(sent.entities, ["imu/z", "imu/n"]);
        assert_eq!(sent.timelines, ["t", "t_sensor"]);
        assert_eq!(sent.skipped, ["recording"]);
        rec.flush_blocking().unwrap();
        // One chunk per series, after the recording's own messages.
        let chunks = storage
            .take()
            .into_iter()
            .filter(|m| matches!(m, rerun::log::LogMsg::ArrowMsg(..)))
            .count();
        assert!(chunks >= 2, "{chunks} chunks");
    }

    #[tokio::test]
    async fn needs_t() {
        let df = SessionContext::new().sql("SELECT 1 AS x").await.unwrap();
        let (rec, _) = RecordingStreamBuilder::new("test").memory().unwrap();
        let e = send_rows(&rec, df, None).await.unwrap_err();
        assert_eq!(e.kind(), ErrorKind::QueryFailed);
    }
}
