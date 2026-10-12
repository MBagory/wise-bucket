//! Table-like files (CSV, JSON Lines, Parquet, Arrow IPC), read by DataFusion. Each
//! becomes one stream, `table`, with the time column guessed from the names.

use std::path::Path;

use datafusion::arrow::datatypes::{DataType, TimeUnit};
use datafusion::datasource::file_format::options::{ArrowReadOptions, JsonReadOptions};
use datafusion::logical_expr::{Expr, cast, ident, lit};
use datafusion::prelude::{CsvReadOptions, ParquetReadOptions, SessionContext};
use futures::StreamExt;
use wb_core::error::{ErrorKind, Result, err};
use wb_core::roots::RecordingKind;

use crate::TABLE_STREAM;
use crate::columns::{T, TimeSource, detect_time, time_type};
use crate::extract::Extraction;
use crate::sink::StreamSink;

/// The CSV delimiter most used on the first line.
fn sniff_delimiter(path: &Path) -> u8 {
    let mut first = String::new();
    if let Ok(f) = std::fs::File::open(path) {
        use std::io::BufRead;
        let _ = std::io::BufReader::new(f).read_line(&mut first);
    }
    b",;\t"
        .iter()
        .copied()
        .max_by_key(|d| first.bytes().filter(|b| b == d).count())
        .unwrap_or(b',')
}

fn time_expr(src: &TimeSource, data_type: &DataType) -> Expr {
    let ns = DataType::Timestamp(TimeUnit::Nanosecond, None);
    match src {
        TimeSource::Timestamp(c) => cast(ident(c), ns),
        TimeSource::Text(c) => cast(ident(c), ns),
        TimeSource::Numeric { column, per_second } => {
            let factor = 1e9 / per_second;
            let raw = if data_type.is_floating() || factor.fract() != 0.0 {
                cast(
                    cast(ident(column), DataType::Float64) * lit(factor),
                    DataType::Int64,
                )
            } else {
                cast(ident(column), DataType::Int64) * lit(factor as i64)
            };
            cast(raw, ns)
        }
    }
}

pub(crate) async fn read(path: &Path, kind: RecordingKind, out: &Path) -> Result<Extraction> {
    let unreadable = |e: &dyn std::fmt::Display| {
        err(
            ErrorKind::RecordingUnreadable,
            format!("{}: {e}", path.display()),
        )
    };
    let ctx = SessionContext::new();
    let file = path.to_string_lossy().to_string();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let df = match kind {
        RecordingKind::Csv => {
            ctx.read_csv(
                &file,
                CsvReadOptions::new()
                    .has_header(true)
                    .delimiter(sniff_delimiter(path))
                    .file_extension(&ext),
            )
            .await
        }
        RecordingKind::JsonLines => {
            let options = JsonReadOptions {
                file_extension: &ext,
                ..Default::default()
            };
            ctx.read_json(&file, options).await
        }
        RecordingKind::Parquet => {
            ctx.read_parquet(&file, ParquetReadOptions::default().file_extension(&ext))
                .await
        }
        _ => {
            let options = ArrowReadOptions {
                file_extension: &ext,
                ..Default::default()
            };
            ctx.read_arrow(&file, options).await
        }
    }
    .map_err(|e| unreadable(&e))?;

    let schema = df.schema().as_arrow().clone();
    let source = detect_time(&schema);
    let mut exprs = Vec::new();
    let mut sink = StreamSink::new(
        out,
        0,
        TABLE_STREAM,
        &format!("{kind:?}").to_lowercase(),
        "table",
    );
    sink.meta.decoded = true;
    match &source {
        Some(src) => {
            let dt = schema
                .field_with_name(src.column())
                .map(|f| f.data_type().clone())
                .unwrap_or(DataType::Null);
            exprs.push(time_expr(src, &dt).alias(T));
            sink.note(format!("t comes from the column {:?}", src.column()));
        }
        None => {
            exprs.push(cast(lit(datafusion::scalar::ScalarValue::Null), time_type()).alias(T));
            sink.note("no time column found: t is empty");
        }
    }
    for f in schema.fields() {
        if f.name() == T {
            // The original `t` became the time column, or is kept under another name.
            if source.as_ref().map(TimeSource::column) != Some(T) {
                exprs.push(ident(T).alias("t_field"));
            }
            continue;
        }
        exprs.push(ident(f.name()));
    }
    let df = df.select(exprs).map_err(|e| unreadable(&e))?;
    let mut stream = df.execute_stream().await.map_err(|e| unreadable(&e))?;
    while let Some(batch) = stream.next().await {
        let batch = batch.map_err(|e| unreadable(&e))?;
        sink.append(batch)?;
    }
    let mut extraction = Extraction::default();
    if let Some(meta) = sink.finish(out)? {
        extraction.streams.push(meta);
    }
    Ok(extraction)
}
