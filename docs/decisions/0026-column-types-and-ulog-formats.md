# 0026. Column types in results, and the formats of a ULog file

**Date:** 2026-10-14 · **Status:** Accepted

## Context

A second cross-check of `sample.ulg` against a pyulog script found every row count, timestamp, parameter, message and dropout identical. pyulog still gave four things the store did not:

- **Each field's type in one call.** `streams.columns` lists names only; `arrow_typeof()` takes one column at a time. The tester wrote `value <> initial` twice, reading `parameters.initial` as a value when it is a boolean.
- **All 103 message formats** the file defines. The store kept the 15 with data; the 88 others include `sensor_baro`, while the log holds four errors "no barometer found".
- **File flags:** `file_corruption`, `has_data_appended`, `has_default_parameters`.
- **Raw microsecond timestamps**, and `write_ulog` to rewrite a log. Times since boot also read as dates (`1970-01-01T00:01:52` for 112.5 s).

The criterion is the agent's: find the information where it already looks, in few queries and few tokens.

## Decision

- **Every `sql` result carries `types`**, the DataFusion type of each column, next to `columns` (`Float64`, `Boolean`, `Timestamp(ns)`, `List(Float64)`: the names of `arrow_typeof` and of its errors). `SELECT * FROM stream('x') LIMIT 0` gives a stream's columns with their types.
- **A ULog file gets a `formats` stream** (`ulog formats`): one row per message format of the definitions section, with `name`, `fields` as declared and `logged`, false for message types without data. It is timed at the header's start. The `stream()` error for a missing stream points to it.
- **No note means none.** `recordings.notes` already reports a file cut or damaged and the crash dumps that appended data holds; defaults are the `system_default` and `airframe_default` columns. The descriptions say that the absence of a note means none of these.
- **Seconds since boot** are `date_part('epoch', t)`, in the description of `t`; the instructions say to give times before 2000 in seconds.
- **The record streams of a ULog file are not flagged for a frozen time:** their rows share the time pyulog gives them (formats, the lines of a text, parameters without changes). Message streams still are (`commander_state`).
- **`parameters.initial`** is described as true for the values at the start of the log, false for changes during it.

## Alternatives

- **Types inline in `streams.columns`** (`initial Boolean`): the largest demo ULog has 35 KB of column names over 99 streams; types add about 18 KB and push `SELECT * FROM streams WHERE recording = …` past the 60 KB cap.
- **A `columns` table:** a fifth table to think of, for a need that one query covers.
- **A recipe only** (`arrow_typeof` per column): the agent lists the columns itself.
- **Unlogged message types as empty rows in `streams`:** `sample.ulg` goes from 18 to about 100 rows in the agent's usual second query.
- **Formats in `properties`:** about 100 rows per ULog in a table the agent reads before concluding, and 30 to 50 KB more in a `recording.json` that every query reads.
- **The flag bits as properties:** they repeat the notes.
- **Telling "subscribed, never published" from "never subscribed":** it needs the `A` messages of the data section, a full read of the file.
- **`write_ulog`:** recordings are never modified ([0012](0012-trade-storage-for-query-speed.md)); export is a possible feature ([0014](0014-possible-features-instead-of-milestones.md)).

## Consequences

- Every recording is extracted again (`EXTRACTOR_VERSION` 4).
- A ULog file's definitions section is read once more, by Wise Bucket itself: px4_ulog keeps the formats to itself.
- Every result is longer by one short type per column.
- px4_ulog stops at a damaged record where pyulog skips it and reads on; that difference remains.
