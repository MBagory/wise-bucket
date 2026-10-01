//! Public sample recordings, one set per log format, for the `demo` command and
//! the format tests.
//!
//! The repository never contains these files: they are downloaded on demand from
//! their upstream repositories at a pinned commit and verified by SHA-256, so each
//! one stays under its own license.

use std::path::Path;

use crate::error::Result;
use crate::fsutil;

/// One format's sample set.
#[derive(Debug)]
pub struct Format {
    /// Short name used on the command line (`demo ulog`).
    pub key: &'static str,
    pub description: &'static str,
    /// SPDX license(s) of the upstream files.
    pub license: &'static str,
    pub files: &'static [SampleFile],
}

#[derive(Debug)]
pub struct SampleFile {
    /// Path inside the format's folder.
    pub path: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl Format {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Raw file in a GitHub repository at a pinned commit.
macro_rules! raw {
    ($repo:literal, $rev:literal, $path:literal) => {
        concat!(
            "https://raw.githubusercontent.com/",
            $repo,
            "/",
            $rev,
            "/",
            $path
        )
    };
}

/// Git LFS file in a GitHub repository at a pinned commit.
macro_rules! lfs {
    ($repo:literal, $rev:literal, $path:literal) => {
        concat!(
            "https://media.githubusercontent.com/media/",
            $repo,
            "/",
            $rev,
            "/",
            $path
        )
    };
}

pub const FORMATS: &[Format] = &[
    Format {
        key: "ros2-mcap",
        description: "ROS 2 bag folder, MCAP storage (talker demo: /topic, /rosout)",
        license: "Apache-2.0",
        files: &[
            SampleFile {
                path: "talker/metadata.yaml",
                url: raw!(
                    "ros2/rosbag2",
                    "b3a42339376e8cae82e292081b2378a800152905",
                    "rosbag2_py/test/resources/mcap/talker/metadata.yaml"
                ),
                size: 1654,
                sha256: "4aa2b1e758564c41618993471280c24d110949e40a9ff815b9da3913e648b6ea",
            },
            SampleFile {
                path: "talker/talker.mcap",
                url: raw!(
                    "ros2/rosbag2",
                    "b3a42339376e8cae82e292081b2378a800152905",
                    "rosbag2_py/test/resources/mcap/talker/talker.mcap"
                ),
                size: 12880,
                sha256: "134c570d7713025b41cfc448519b376373e0c78bd57debadcb30186eaa3b111d",
            },
        ],
    },
    Format {
        key: "ros2-db3",
        description: "ROS 2 bag folder, SQLite storage (same talker demo)",
        license: "Apache-2.0",
        files: &[
            SampleFile {
                path: "talker/metadata.yaml",
                url: raw!(
                    "ros2/rosbag2",
                    "b3a42339376e8cae82e292081b2378a800152905",
                    "rosbag2_py/test/resources/sqlite3/talker/metadata.yaml"
                ),
                size: 1595,
                sha256: "3e20c86e00a0fee096044e107d2a57b7c342915c47274dfb4a1877a9e1202fdc",
            },
            SampleFile {
                path: "talker/talker.db3",
                url: raw!(
                    "ros2/rosbag2",
                    "b3a42339376e8cae82e292081b2378a800152905",
                    "rosbag2_py/test/resources/sqlite3/talker/talker.db3"
                ),
                size: 28672,
                sha256: "2753e17c8ffeb50fd44d1768014f0fadc05fc184006c2c042abb7fdbef52e084",
            },
        ],
    },
    Format {
        key: "mcap",
        description: "Loose MCAP files (ROS 1-encoded demo, spec conformance file)",
        license: "MIT",
        files: &[
            SampleFile {
                path: "demo.mcap",
                url: lfs!(
                    "foxglove/mcap",
                    "60e73f4b4ec065c26324d71b57dfb53085008528",
                    "testdata/mcap/demo.mcap"
                ),
                size: 990,
                sha256: "b90a0af442bfc6aaf31d7ae0a361f04d2a4eecca7168f9917391b74697af5c8c",
            },
            SampleFile {
                path: "TenMessages-ch-chx-mx-pad-rch-rsh-st-sum.mcap",
                url: lfs!(
                    "foxglove/mcap",
                    "60e73f4b4ec065c26324d71b57dfb53085008528",
                    "tests/conformance/data/TenMessages/TenMessages-ch-chx-mx-pad-rch-rsh-st-sum.mcap"
                ),
                size: 1083,
                sha256: "358413d53e6fb2753efb3eac682bc9d776bb2afb1de5e395817efd8f16303fbc",
            },
        ],
    },
    Format {
        key: "ros1-bag",
        description: "ROS 1 bags (demo, and one bag in three compressions)",
        license: "MIT",
        files: &[
            SampleFile {
                path: "demo.bag",
                url: lfs!(
                    "foxglove/mcap",
                    "60e73f4b4ec065c26324d71b57dfb53085008528",
                    "testdata/bags/demo.bag"
                ),
                size: 5295,
                sha256: "7792eacb1f0e8f7b7123ce127a1115c818f1826244cd7239f538da6133f65568",
            },
            SampleFile {
                path: "noetic-multitopic-none.bag",
                url: lfs!(
                    "foxglove/mcap",
                    "60e73f4b4ec065c26324d71b57dfb53085008528",
                    "testdata/bags/generated/noetic-multitopic-none.bag"
                ),
                size: 5482,
                sha256: "a5a7acb5f8af7c5d1bbfe2a4dbc2b3e411abc83284019c0c63c570a49f8d9f1e",
            },
            SampleFile {
                path: "noetic-multitopic-lz4.bag",
                url: lfs!(
                    "foxglove/mcap",
                    "60e73f4b4ec065c26324d71b57dfb53085008528",
                    "testdata/bags/generated/noetic-multitopic-lz4.bag"
                ),
                size: 5381,
                sha256: "2399908bc2ad38b59e3f400511d397acac4078b440dd73a0e62fd1c002a0b5ef",
            },
            SampleFile {
                path: "noetic-multitopic-bz2.bag",
                url: lfs!(
                    "foxglove/mcap",
                    "60e73f4b4ec065c26324d71b57dfb53085008528",
                    "testdata/bags/generated/noetic-multitopic-bz2.bag"
                ),
                size: 5349,
                sha256: "fe84b3c9d29b1755795d4df9474975064e25cfe66a0717b55fbf88583b3c67a8",
            },
        ],
    },
    Format {
        key: "ulog",
        description: "PX4 ULog, 69 s bench log of a disarmed autopilot",
        license: "BSD-3-Clause",
        files: &[SampleFile {
            path: "sample.ulg",
            url: raw!(
                "PX4/pyulog",
                "ffbe3755d903e93797a89fb4fce26e0c0420a42f",
                "test/sample.ulg"
            ),
            size: 4053364,
            sha256: "81952e6059bc095717e7911c010e07f85749d6b04d332a5ffc51575a3fd0a558",
        }],
    },
    Format {
        key: "dataflash",
        description: "ArduPilot DataFlash log (ArduPlane SITL)",
        license: "LGPL-3.0",
        files: &[SampleFile {
            path: "test.BIN",
            url: raw!(
                "ArduPilot/pymavlink",
                "c6b7a5100bb71adcf899bcf667236b5ea8f4cf45",
                "tests/test.BIN"
            ),
            size: 65536,
            sha256: "a51f040b2ad7f55185c1da5705f226f51469b86562c4ff76d05c99e0e52c2bf4",
        }],
    },
    Format {
        key: "tlog",
        description: "MAVLink 2 telemetry log (one BATTERY_STATUS, truncated tail)",
        license: "LGPL-3.0",
        files: &[SampleFile {
            path: "capture.mav2.battery_status.tlog",
            url: raw!(
                "ArduPilot/pymavlink",
                "c6b7a5100bb71adcf899bcf667236b5ea8f4cf45",
                "generator/javascript/test/capture.mav2.battery_status.tlog"
            ),
            size: 70,
            sha256: "8e23bfc5074d262989d74fea006851e4691f6201de67af081a342745bae0bd7a",
        }],
    },
    Format {
        key: "can",
        description: "CAN logs (Vector ASC with error frames, BLF) and a DBC database",
        license: "LGPL-3.0 (ASC, BLF), MIT (DBC)",
        files: &[
            SampleFile {
                path: "logfile.asc",
                url: raw!(
                    "hardbyte/python-can",
                    "b4f82abede25ff83376be793a2935c41f81c3869",
                    "test/data/logfile.asc"
                ),
                size: 3571,
                sha256: "e3501125412852bdfa62b59dab51dc09009d92a6b1215e7a86f01fc5bef4549b",
            },
            SampleFile {
                path: "test_CanMessage.blf",
                url: raw!(
                    "hardbyte/python-can",
                    "b4f82abede25ff83376be793a2935c41f81c3869",
                    "test/data/test_CanMessage.blf"
                ),
                size: 420,
                sha256: "c04d0cb0ed2a34668e643687d9e815e0d5e395746702d2e0b82e286ca6529415",
            },
            SampleFile {
                path: "socialledge.dbc",
                url: raw!(
                    "cantools/cantools",
                    "86d15e08ea34dc1a2282907bfa3d87c988018115",
                    "tests/files/dbc/socialledge.dbc"
                ),
                size: 2808,
                sha256: "e3769d6b84eb3df186492690f26da314f233b6ea474a7f1a1edd960d7f27bff4",
            },
        ],
    },
    Format {
        key: "mdf4",
        description: "ASAM MDF 4.10 measurement file",
        license: "LGPL-3.0",
        files: &[SampleFile {
            path: "test_batch.mf4",
            url: raw!(
                "danielhrisca/asammdf",
                "8491873b5fe8128c2f752c3b7d1df6ec696e541f",
                "test/asammdf/gui/resources/test_batch.mf4"
            ),
            size: 8760,
            sha256: "a8385e35f64ad46c617c3dbaf109efe8b7b941884210fcb16e9ceafa1167f266",
        }],
    },
    Format {
        key: "parquet",
        description: "Apache Parquet table (the format of Wise Bucket's decoded cache)",
        license: "Apache-2.0",
        files: &[SampleFile {
            path: "alltypes_plain.parquet",
            url: raw!(
                "apache/parquet-testing",
                "56653c437c8092f704a092d0d1d4e600124cd49f",
                "data/alltypes_plain.parquet"
            ),
            size: 1851,
            sha256: "12a618d20a59ee0967fef45e7ec1ff6d451e724838edc1bbeac780ca15e8fcc4",
        }],
    },
];

/// Format keys, in catalogue order.
pub fn keys() -> impl Iterator<Item = &'static str> {
    FORMATS.iter().map(|f| f.key)
}

pub fn find(key: &str) -> Option<&'static Format> {
    FORMATS.iter().find(|f| f.key == key)
}

/// Downloads a format's files into `dest/<key>/`, skipping files already there
/// with the right checksum.
pub fn fetch(format: &Format, dest: &Path, progress: &dyn Fn(&str)) -> Result<()> {
    for f in format.files {
        fsutil::download_verified(
            f.url,
            &dest.join(format.key).join(f.path),
            f.sha256,
            progress,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_consistent() {
        let mut keys: Vec<_> = keys().collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), FORMATS.len(), "duplicate format key");
        for f in FORMATS {
            for s in f.files {
                assert_eq!(s.sha256.len(), 64, "{}", s.path);
            }
        }
    }
}
