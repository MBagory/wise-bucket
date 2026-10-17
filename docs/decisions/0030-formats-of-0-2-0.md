# 0030. The crates of 0.2.0, and the formats still waiting

**Date:** 2026-10-17 · **Status:** Accepted (updates the formats of [0014](0014-possible-features-instead-of-milestones.md) and the crates of [0016](0016-formats-need-an-external-rust-decoder.md))

## Context

[0016](0016-formats-need-an-external-rust-decoder.md) reads a format only when an external Rust crate decodes it, and lists the others as possible features ([0014](0014-possible-features-instead-of-milestones.md)). A crates.io check on 2026-10-14 found crates for several of them, including one that decodes CAN frames with a DBC file. Each crate was tested on a public sample against a reference tool before its reader was written.

## Decision

- **Crates added**, each with the reference its values match:
  - `blf_asc`: Vector CAN logs, ASC and BLF (python-can).
  - `falcon_mdf`: CAN frames decoded with a DBC file ([0029](0029-can-signals-from-confirmed-dbc.md); cantools), and MDF4 files (asammdf).
  - `dflog`: ArduPilot DataFlash logs (pymavlink).
  - `prost-reflect`: MCAP protobuf messages (`mcap-protobuf-support`).
  - `nmea`: NMEA 0183 sentences (pynmea2).
  - `canboat`: NMEA 2000 captures in canboat's PLAIN format (canboat's C analyzer).
- **Encoded messages** (CAN with a DBC, NMEA 2000, NMEA 0183) keep their raw stream next to the decoded ones; messages no decoder knows stay queryable there.
- **Possible formats, with a crate:** HDF5 and MATLAB v7.3 (`hdf5-pure`; waiting for a real file to design the layout on), SocketCAN candump (`candump-parse`), Betaflight Blackbox (`blackbox-log`), DJI flight logs (`dji-log-parser`; newer logs need a DJI key), u-blox UBX (`ublox`), RINEX (`rinex`), GPX (`gpx`), KML (`kml`), MATLAB v5 (`matfile`), Rerun `.rrd` (`rerun`), WAV (`hound`), NumPy (`npyz`), Zarr (`zarrs`), TFRecord (`tfrecord`), InfluxDB line protocol (`influxdb-line-protocol`), NetCDF (`netcdf`, C library), PCAP (`pcap-parser`), CCSDS packets (`spacepackets`).
- **Possible formats, without a crate:** NI TDMS with DAQmx raw data, the layout NI DAQ logging writes (`tdms-rs` refuses such a file, `tedium` its DAQmx channels), ROS 1 messages (bags and ROS 1-encoded MCAP), LCM, Apollo Cyber RT `.record`, eCAL measurements, RTI Connext recordings, PEAK `.trc`, Kvaser `.kme`, ADTF `.dat`, RTMaps `.rec`, Tektronix `.wfm`/`.isf`, LeCroy `.trc`, Saleae `.sal`, XTCE (CCSDS parameter definitions), LabVIEW `.lvm`, plain-text logs (glog, spdlog, syslog, journald files).

## Alternatives

- **`dbc-rs`** for DBC files: it rejected the four real vehicle DBCs tried (comma.ai opendbc).
- **`can_decode`** for DBC files: it decodes multiplexed signals whatever the multiplexer's value.
- **`ardupilot-binlog`** for DataFlash: it skips messages with half-float fields and leaves latitudes and longitudes unscaled.
- **`autors-asc`** for ASC: it rejects a whole file on a remote frame written without a length.

## Consequences

- DBC decoding no longer waits for a crate.
- `falcon_mdf` pins `quick-xml` 0.37, whose advisories RUSTSEC-2026-0194 and 0195 `deny.toml` ignores: the first is reached only by MDF4 metadata, in the low-priority extraction process stopped after an hour, the second never.
- Building from source needs Rust 1.98 (`dflog`).
- A format moves to the first list when a crate appears for it, and out of these lists when it ships.
