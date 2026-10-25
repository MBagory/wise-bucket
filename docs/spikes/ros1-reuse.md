# Spike: reusing existing crates for ROS 1 bags

**Date:** 2026-10-18 · **Outcome:** ❌ not reused: `rosbag` 0.6.3 and `rosrust` 0.9.12 decode correctly, but fail cargo-deny and panic on truncated bags.

## Question

Can wise-bucket read ROS 1 bags with existing crates instead of a new decoder crate? `rosbag` reads the container, and `rosrust::DynamicMsg` decodes a payload from its type name and definition. No other Rust crate decodes ROS 1 messages from a definition read at runtime: `apex-io` has fixed types, `roslibrust` generates types at compile time, and the rest are ROS 2 only.

## What was tested

1. **Build:** `rosbag = "0.6.3"` with `rosrust = "0.9.12"` builds without ROS installed: 207 crates, 59 s debug build. `rosbag` needs a C compiler (`lz4-sys`, `bzip2-sys`) and memory-maps the file.
2. **cargo-deny** with wise-bucket's `deny.toml`:
   - 4 vulnerabilities: hyper 0.10 (RUSTSEC-2021-0078, RUSTSEC-2021-0079), time 0.1 (RUSTSEC-2020-0071) and idna 0.1 (RUSTSEC-2024-0421).
   - 6 unmaintained crates: yaml-rust, multipart, buf_redux, safemem, traitobject and twoway.
   - All of them come from rosrust's ROS client (xml-rpc), which decoding does not use. `rosbag` alone adds no advisory, and no license fails.
3. **Upstream fixtures** (foxglove/mcap `60e73f4b`, `testdata/bags/`): `demo.bag` and `noetic-multitopic-{none,bz2,lz4}.bag` decode to exactly the expected connections, times and values. `noetic-empty.bag` yields nothing.
4. **All types:** [`ros1-reuse/generate.py`](ros1-reuse/generate.py) runs in `ros:noetic-ros-base` and writes two files:
   - `types.bag`: 13 topics and 17 messages, covering Imu, JointState, TFMessage, DiagnosticArray, NavSatFix, Marker, Log, Image, String, Empty, and a custom type with every builtin, array kind and extreme value. It is 51 375 bytes, SHA-256 `c57da51542f9eb617a8b677009fae55db04a4e87d13606a92bfb7a89759002da`.
   - `types.expected.json`: the values read back by ROS's own deserializer.

   `DynamicMsg` matches all 17 messages exactly, with no bytes left over, once the `Empty` problem below is worked around. The script was rebuilt from the spike session after its scratch copy was lost, and has not been run since.
5. **Problems found:**
   - `rosbag` panics on a bag cut inside a chunk, and on a crashed recording whose header still says `index_pos=0`: `chunk_records` slices the file up to `index_pos`. A bag cut exactly at the index reads fine.
   - `DynamicMsg` rejects a definition with an empty dependency section (`MSG: std_msgs/Empty`), so any message with a `std_msgs/Empty` field fails.
   - `DynamicMsg` accepts bytes left after the last field; the caller can compare the cursor position. It also rejects type names that break REP 144, such as a one-letter package.
   - On short payloads and huge length prefixes, `DynamicMsg` returns an error without allocating.
6. **Generator pitfalls (ROS Noetic):**
   - `Bag.write(connection_header=…)` replaces the whole connection header, so it must also carry `type`, `md5sum` and `message_definition`.
   - The Python `rosbag` keeps one connection per topic. It stores a second type on the same topic under the first connection, and genpy then decodes it into wrong values without an error. Two connections on one topic need a hand-built fixture.

## Decision

The ROS 1 decoder is written as its own crate, `rosbag1`: a streaming reader without C, tested against ROS's own deserializer through `generate.py`.

## Possible improvements

- Upstream fixes to `rosbag` for truncated and unindexed bags would make its container reader reusable.
- A `ros_message` decoder without rosrust's network stack would let the reuse path pass cargo-deny.
- An upstream fix to `DynamicMsg` for empty dependency sections would decode `std_msgs/Empty` fields.
- `generate.py` could also write bz2 and lz4 copies of `types.bag`, and a bag left unindexed by a stopped recorder.
