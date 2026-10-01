//! Managed PostgreSQL + pgvector: provisioning and process lifecycle.
//!
//! * PostgreSQL binaries come from the `theseus-rs/postgresql-binaries` releases
//!   (the same builds used by the `postgresql_embedded` crate), pinned by version
//!   and SHA-256.
//! * pgvector is built from its pinned, checksum-verified source release against
//!   those exact binaries (PGXS), with `PG_SYSROOT` set to the local SDK on macOS
//!   and `OPTFLAGS=""` for a portable build.
//! * The server listens **only** on a Unix socket in a private directory; there
//!   is no TCP listener.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};
use sqlx::postgres::PgConnectOptions;

use crate::error::{ErrorKind, IoContext, Result, WbError, err};
use crate::fsutil;

/// Pinned PostgreSQL version (theseus-rs release tag).
pub const PG_VERSION: &str = "17.11.0";
/// Pinned pgvector version.
pub const PGVECTOR_VERSION: &str = "0.8.6";
/// SHA-256 of `https://github.com/pgvector/pgvector/archive/refs/tags/v0.8.6.tar.gz`.
pub const PGVECTOR_SHA256: &str =
    "10bf9938906e5d643bbc4a7eea104b6f57ba4898e5b76b20e60484ea1d5a7f8f";

/// SHA-256 of each supported PostgreSQL archive.
const PG_ARCHIVES: &[(&str, &str)] = &[
    (
        "x86_64-apple-darwin",
        "e43a81b15e1cfe7f9d8fd79c6d4d0366e9001a5f690e322224dca704656602f7",
    ),
    (
        "aarch64-apple-darwin",
        "fd4b62794b160e26973a768a1eef3248aef9d2ff23ebd6d884a4299485e28e57",
    ),
    (
        "x86_64-unknown-linux-gnu",
        "b7a1ba6bae6499d8296e3e81b0171eecfd1766ca9aaa0057e41ad3e844e5e2e0",
    ),
    (
        "aarch64-unknown-linux-gnu",
        "abffda09209280ec1502b73720dc4d254fb7fff9a072e324926c600a5b16c221",
    ),
];

/// Database, roles and socket settings.
pub const DB_NAME: &str = "wisebucket";
pub const SUPERUSER: &str = "postgres";
pub const APP_ROLE: &str = "wb_app";
pub const READER_ROLE: &str = "wb_reader";
/// Only used to name the socket file (`.s.PGSQL.5432`); nothing listens on TCP.
pub const SOCKET_PORT: u16 = 5432;
/// Conservative limit for `sun_path` (104 bytes on macOS, 108 on Linux).
const MAX_SOCKET_PATH: usize = 100;

/// Progress reporting callback (messages go to stderr in the CLI).
pub type Progress<'a> = &'a dyn Fn(&str);

/// Target triple used to pick PostgreSQL binaries.
pub fn target() -> Result<&'static str> {
    let t = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        (os, arch) => {
            return Err(err(
                ErrorKind::UnsupportedPlatform,
                format!("managed PostgreSQL is not available for {os}/{arch}"),
            ));
        }
    };
    Ok(t)
}

fn archive_sha256(target: &str) -> Result<&'static str> {
    PG_ARCHIVES
        .iter()
        .find(|(t, _)| *t == target)
        .map(|(_, h)| *h)
        .ok_or_else(|| {
            err(
                ErrorKind::UnsupportedPlatform,
                format!("no pinned archive for {target}"),
            )
        })
}

/// Credentials of the managed cluster, stored in a 0600 file in the state directory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Secrets {
    pub superuser_password: String,
    pub app_password: String,
}

/// Paths and operations of the managed cluster for one state directory.
#[derive(Debug, Clone)]
pub struct Managed {
    pub state_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl Managed {
    pub fn new(state_dir: &Path, runtime_dir: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
            runtime_dir: runtime_dir.to_path_buf(),
        }
    }

    pub fn pg_home(&self) -> Result<PathBuf> {
        Ok(self
            .runtime_dir
            .join(format!("postgresql-{PG_VERSION}-{}", target()?)))
    }

    pub fn bin(&self, name: &str) -> Result<PathBuf> {
        Ok(self.pg_home()?.join("bin").join(name))
    }

    pub fn data_dir(&self) -> PathBuf {
        self.state_dir.join("pg")
    }

    pub fn secrets_path(&self) -> PathBuf {
        self.state_dir.join("secrets").join("db.toml")
    }

    pub fn log_path(&self) -> PathBuf {
        self.state_dir.join("logs").join("postgres.log")
    }

    /// Serializes start/stop decisions between processes.
    pub fn lifecycle_lock_path(&self) -> PathBuf {
        self.state_dir.join("lifecycle.lock")
    }

    /// Directory holding the Unix socket: `<state>/run`, or a short directory
    /// under the temp dir if that path would exceed the socket length limit.
    pub fn socket_dir(&self) -> Result<PathBuf> {
        let socket_name = format!(".s.PGSQL.{SOCKET_PORT}");
        let preferred = self.state_dir.join("run");
        if preferred.join(&socket_name).as_os_str().len() <= MAX_SOCKET_PATH {
            return Ok(preferred);
        }
        let digest = Sha256::digest(self.state_dir.to_string_lossy().as_bytes());
        let short = PathBuf::from("/tmp").join(format!("wb-{}", &hex::encode(digest)[..12]));
        if short.join(&socket_name).as_os_str().len() <= MAX_SOCKET_PATH {
            return Ok(short);
        }
        Err(err(
            ErrorKind::SocketPathTooLong,
            format!("{} is too long for a Unix socket", preferred.display()),
        ))
    }

    // ------------------------------------------------------------------
    // Provisioning
    // ------------------------------------------------------------------

    pub fn is_runtime_installed(&self) -> bool {
        self.pg_home()
            .map(|h| h.join("bin/postgres").is_file() && h.join(".wb-installed").is_file())
            .unwrap_or(false)
    }

    pub fn installed_pgvector_version(&self) -> Option<String> {
        let home = self.pg_home().ok()?;
        let control = std::fs::read_to_string(home.join("share/extension/vector.control")).ok()?;
        let lib_ok = ["lib/vector.so", "lib/vector.dylib"]
            .iter()
            .any(|l| home.join(l).is_file());
        if !lib_ok {
            return None;
        }
        control.lines().find_map(|l| {
            let l = l.trim();
            l.strip_prefix("default_version").map(|v| {
                v.trim_start_matches([' ', '='])
                    .trim()
                    .trim_matches('\'')
                    .to_string()
            })
        })
    }

    pub fn is_pgvector_installed(&self) -> bool {
        self.installed_pgvector_version().as_deref() == Some(PGVECTOR_VERSION)
    }

    pub fn is_initialized(&self) -> bool {
        self.data_dir().join("PG_VERSION").is_file() && self.secrets_path().is_file()
    }

    /// Downloads and unpacks the pinned PostgreSQL binaries (idempotent).
    pub fn install_runtime(&self, progress: Progress) -> Result<()> {
        // The runtime directory may be shared between state directories and concurrent setups.
        let _lock = fsutil::FileLock::acquire(&self.runtime_dir.join("install.lock"))?;
        if self.is_runtime_installed() {
            progress(&format!("PostgreSQL {PG_VERSION} already installed"));
            return Ok(());
        }
        let target = target()?;
        let name = format!("postgresql-{PG_VERSION}-{target}.tar.gz");
        let url = format!(
            "https://github.com/theseus-rs/postgresql-binaries/releases/download/{PG_VERSION}/{name}"
        );
        let downloads = self.runtime_dir.join("downloads");
        let archive = downloads.join(&name);
        fsutil::download_verified(&url, &archive, archive_sha256(target)?, progress)?;

        progress("Unpacking PostgreSQL");
        let staging = self
            .runtime_dir
            .join(format!(".staging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        unpack_tar_gz(&archive, &staging)?;
        let home = self.pg_home()?;
        let unpacked = staging.join(home.file_name().unwrap_or_default());
        if !unpacked.join("bin/postgres").is_file() {
            return Err(err(
                ErrorKind::DownloadFailed,
                format!("unexpected archive layout in {}", archive.display()),
            ));
        }
        let _ = std::fs::remove_dir_all(&home);
        std::fs::rename(&unpacked, &home).io_ctx(|| format!("install {}", home.display()))?;
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::write(home.join(".wb-installed"), PG_VERSION)?;
        progress(&format!(
            "PostgreSQL {PG_VERSION} installed in {}",
            home.display()
        ));
        Ok(())
    }

    /// Builds and installs the pinned pgvector release into the managed PostgreSQL (idempotent).
    pub fn install_pgvector(&self, progress: Progress) -> Result<()> {
        let _lock = fsutil::FileLock::acquire(&self.runtime_dir.join("install.lock"))?;
        if self.is_pgvector_installed() {
            progress(&format!("pgvector {PGVECTOR_VERSION} already installed"));
            return Ok(());
        }
        check_toolchain()?;
        let url = format!(
            "https://github.com/pgvector/pgvector/archive/refs/tags/v{PGVECTOR_VERSION}.tar.gz"
        );
        let archive = self
            .runtime_dir
            .join("downloads")
            .join(format!("pgvector-{PGVECTOR_VERSION}.tar.gz"));
        fsutil::download_verified(&url, &archive, PGVECTOR_SHA256, progress)?;

        let build_root = self.runtime_dir.join("build");
        let src = build_root.join(format!("pgvector-{PGVECTOR_VERSION}"));
        let _ = std::fs::remove_dir_all(&src);
        unpack_tar_gz(&archive, &build_root)?;
        let log_path = build_root.join(format!("pgvector-{PGVECTOR_VERSION}-build.log"));
        let pg_config = self.bin("pg_config")?;

        progress(&format!(
            "Building pgvector {PGVECTOR_VERSION} (about 15 s)"
        ));
        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let mut make_args = vec![
            format!("PG_CONFIG={}", pg_config.display()),
            // Portable build: no -march=native, so the binary survives CPU changes.
            "OPTFLAGS=".to_string(),
        ];
        if cfg!(target_os = "macos") {
            // PGXS records the SDK path of the machine that built PostgreSQL; use ours.
            if let Some(sdk) = command_stdout("xcrun", &["--show-sdk-path"]) {
                make_args.push(format!("PG_SYSROOT={sdk}"));
            }
        }
        let mut log = std::fs::File::create(&log_path)?;
        for step in [vec![format!("-j{jobs}")], vec!["install".to_string()]] {
            let out = Command::new("make")
                .current_dir(&src)
                .args(&step)
                .args(&make_args)
                .output()
                .map_err(|e| err(ErrorKind::ToolchainMissing, format!("cannot run make: {e}")))?;
            log.write_all(&out.stdout)?;
            log.write_all(&out.stderr)?;
            if !out.status.success() {
                return Err(err(
                    ErrorKind::PgvectorBuildFailed,
                    format!(
                        "`make {}` failed; see {}",
                        step.join(" "),
                        log_path.display()
                    ),
                ));
            }
        }
        if !self.is_pgvector_installed() {
            return Err(err(
                ErrorKind::PgvectorBuildFailed,
                format!(
                    "pgvector files missing after install; see {}",
                    log_path.display()
                ),
            ));
        }
        progress(&format!("pgvector {PGVECTOR_VERSION} installed"));
        Ok(())
    }

    /// Creates the cluster (`initdb`) with generated passwords (idempotent).
    pub fn init_cluster(&self, progress: Progress) -> Result<()> {
        if self.is_initialized() {
            progress("Database cluster already initialized");
            return Ok(());
        }
        let data = self.data_dir();
        if data.exists() && !data.join("PG_VERSION").is_file() {
            let _ = std::fs::remove_dir_all(&data);
        }
        if data.join("PG_VERSION").is_file() && !self.secrets_path().is_file() {
            return Err(err(
                ErrorKind::DbStartFailed,
                format!(
                    "{} exists but its credentials file {} is missing; move the data directory away and re-run setup",
                    data.display(),
                    self.secrets_path().display()
                ),
            ));
        }
        let secrets = Secrets {
            superuser_password: random_secret()?,
            app_password: random_secret()?,
        };
        let pwfile = self.state_dir.join("secrets").join(".initdb-pw");
        fsutil::write_private(&pwfile, secrets.superuser_password.as_bytes())?;
        progress("Initializing the database cluster");
        let out = Command::new(self.bin("initdb")?)
            .arg("-D")
            .arg(&data)
            .args([
                "-U",
                SUPERUSER,
                "--auth=scram-sha-256",
                "-E",
                "UTF8",
                "--locale=C",
                "--no-instructions",
            ])
            .arg(format!("--pwfile={}", pwfile.display()))
            .output()
            .map_err(|e| err(ErrorKind::DbStartFailed, format!("cannot run initdb: {e}")))?;
        let _ = std::fs::remove_file(&pwfile);
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&data);
            return Err(err(
                ErrorKind::DbStartFailed,
                format!(
                    "initdb failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ));
        }
        // Wise Bucket settings live in their own file, rewritten on every start.
        let mut conf = std::fs::OpenOptions::new()
            .append(true)
            .open(data.join("postgresql.conf"))?;
        writeln!(
            conf,
            "\n# Wise Bucket: managed settings\ninclude_if_exists = 'wisebucket.conf'"
        )?;
        fsutil::write_private(
            &self.secrets_path(),
            toml::to_string(&secrets)
                .map_err(|e| err(ErrorKind::Internal, e.to_string()))?
                .as_bytes(),
        )?;
        Ok(())
    }

    pub fn secrets(&self) -> Result<Secrets> {
        let text = std::fs::read_to_string(self.secrets_path()).map_err(|_| {
            err(
                ErrorKind::DbNotInitialized,
                "managed database credentials not found",
            )
        })?;
        toml::from_str(&text).map_err(|e| err(ErrorKind::ConfigInvalid, e.to_string()))
    }

    // ------------------------------------------------------------------
    // Process lifecycle
    // ------------------------------------------------------------------

    /// `true` if the managed PostgreSQL server is running.
    pub fn is_running(&self) -> Result<bool> {
        if !self.data_dir().join("PG_VERSION").is_file() {
            return Ok(false);
        }
        let status = Command::new(self.bin("pg_ctl")?)
            .arg("-D")
            .arg(self.data_dir())
            .arg("status")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| err(ErrorKind::DbStartFailed, format!("cannot run pg_ctl: {e}")))?;
        Ok(status.success())
    }

    /// Starts the server (no-op if already running).
    pub fn start(&self) -> Result<()> {
        if !self.is_initialized() {
            return Err(err(
                ErrorKind::DbNotInitialized,
                "run `wise-bucket-server setup` first",
            ));
        }
        if self.is_running()? {
            return Ok(());
        }
        let socket_dir = self.socket_dir()?;
        fsutil::ensure_private_dir(&socket_dir)?;
        #[cfg(unix)]
        {
            // The /tmp fallback is shared: refuse a directory created by someone else.
            use std::os::unix::fs::MetadataExt;
            let mine = std::fs::metadata(&self.state_dir)?.uid();
            if std::fs::metadata(&socket_dir)?.uid() != mine {
                return Err(err(
                    ErrorKind::DbStartFailed,
                    format!(
                        "socket directory {} is owned by another user",
                        socket_dir.display()
                    ),
                ));
            }
        }
        let conf = format!(
            "# Written by Wise Bucket on every start. Do not edit.\n\
             listen_addresses = ''\n\
             port = {SOCKET_PORT}\n\
             unix_socket_directories = '{}'\n\
             unix_socket_permissions = 0700\n",
            socket_dir.display().to_string().replace('\'', "''")
        );
        fsutil::write_private(&self.data_dir().join("wisebucket.conf"), conf.as_bytes())?;
        let log = self.log_path();
        std::fs::create_dir_all(log.parent().unwrap_or(Path::new(".")))?;
        let out = Command::new(self.bin("pg_ctl")?)
            .arg("-D")
            .arg(self.data_dir())
            .arg("-l")
            .arg(&log)
            .args(["-w", "-t", "60", "start"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| err(ErrorKind::DbStartFailed, format!("cannot run pg_ctl: {e}")))?;
        if !out.status.success() {
            return Err(err(
                ErrorKind::DbStartFailed,
                format!(
                    "pg_ctl start failed: {} (see {})",
                    String::from_utf8_lossy(&out.stderr).trim(),
                    log.display()
                ),
            ));
        }
        tracing::info!(data = %self.data_dir().display(), "managed PostgreSQL started");
        Ok(())
    }

    /// Stops the server with a fast shutdown (no-op if not running).
    pub fn stop(&self) -> Result<()> {
        if !self.is_running()? {
            return Ok(());
        }
        let out = Command::new(self.bin("pg_ctl")?)
            .arg("-D")
            .arg(self.data_dir())
            .args(["-m", "fast", "-w", "-t", "60", "stop"])
            .output()
            .map_err(|e| err(ErrorKind::DbStartFailed, format!("cannot run pg_ctl: {e}")))?;
        if !out.status.success() {
            return Err(err(
                ErrorKind::DbStartFailed,
                format!(
                    "pg_ctl stop failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
            ));
        }
        tracing::info!("managed PostgreSQL stopped");
        Ok(())
    }

    // ------------------------------------------------------------------
    // Connections
    // ------------------------------------------------------------------

    fn base_options(&self) -> Result<PgConnectOptions> {
        use sqlx::ConnectOptions;
        Ok(PgConnectOptions::new()
            .socket(self.socket_dir()?)
            .port(SOCKET_PORT)
            .log_slow_statements(
                tracing::log::LevelFilter::Off,
                std::time::Duration::from_secs(60),
            ))
    }

    /// Connection options for the application role.
    pub fn app_options(&self) -> Result<PgConnectOptions> {
        let s = self.secrets()?;
        Ok(self
            .base_options()?
            .username(APP_ROLE)
            .password(&s.app_password)
            .database(DB_NAME))
    }

    /// Connection options for the superuser (bootstrap only).
    pub fn superuser_options(&self, database: &str) -> Result<PgConnectOptions> {
        let s = self.secrets()?;
        Ok(self
            .base_options()?
            .username(SUPERUSER)
            .password(&s.superuser_password)
            .database(database)
            .application_name("wisebucket-setup"))
    }

    /// Creates roles, the database and the extensions (idempotent). Requires a running server.
    pub async fn bootstrap(&self, progress: Progress<'_>) -> Result<()> {
        use sqlx::{Connection, PgConnection};
        let s = self.secrets()?;
        let mut conn = PgConnection::connect_with(&self.superuser_options("postgres")?)
            .await
            .map_err(connect_err)?;
        // Passwords are generated hex strings, so they are safe inside literals.
        let roles = format!(
            "DO $$ BEGIN
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{APP_ROLE}') THEN
                 CREATE ROLE {APP_ROLE} LOGIN PASSWORD '{pw}';
               ELSE
                 ALTER ROLE {APP_ROLE} LOGIN PASSWORD '{pw}';
               END IF;
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{READER_ROLE}') THEN
                 CREATE ROLE {READER_ROLE} NOLOGIN;
               END IF;
             END $$;
             GRANT {READER_ROLE} TO {APP_ROLE};
             GRANT pg_read_all_stats TO {APP_ROLE};",
            pw = s.app_password
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(roles))
            .execute(&mut conn)
            .await
            .map_err(db_err)?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
                .bind(DB_NAME)
                .fetch_one(&mut conn)
                .await
                .map_err(db_err)?;
        if !exists {
            progress("Creating the `wisebucket` database");
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {DB_NAME} OWNER {APP_ROLE} ENCODING 'UTF8'"
            )))
            .execute(&mut conn)
            .await
            .map_err(db_err)?;
        }
        conn.close().await.ok();

        let mut conn = PgConnection::connect_with(&self.superuser_options(DB_NAME)?)
            .await
            .map_err(connect_err)?;
        sqlx::raw_sql(
            "CREATE EXTENSION IF NOT EXISTS vector;
             ALTER EXTENSION vector UPDATE;
             CREATE EXTENSION IF NOT EXISTS pg_trgm;",
        )
        .execute(&mut conn)
        .await
        .map_err(|e| err(ErrorKind::VectorExtensionMissing, e.to_string()))?;
        conn.close().await.ok();
        Ok(())
    }
}

pub(crate) fn connect_err(e: sqlx::Error) -> WbError {
    err(ErrorKind::DbConnectFailed, e.to_string())
}

pub(crate) fn db_err(e: sqlx::Error) -> WbError {
    err(ErrorKind::Internal, format!("database error: {e}"))
}

fn random_secret() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| err(ErrorKind::Internal, e.to_string()))?;
    Ok(hex::encode(bytes))
}

fn command_stdout(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Verifies that `make` and a C compiler are available.
pub fn check_toolchain() -> Result<()> {
    let ok = |cmd: &str| {
        Command::new(cmd)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !ok("make") || !(ok("cc") || ok("gcc") || ok("clang")) {
        return Err(err(
            ErrorKind::ToolchainMissing,
            "`make` and a C compiler (cc/gcc/clang) are needed to build pgvector",
        ));
    }
    Ok(())
}

fn unpack_tar_gz(archive: &Path, into: &Path) -> Result<()> {
    std::fs::create_dir_all(into)?;
    let f = std::fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    ar.set_preserve_permissions(true);
    ar.unpack(into)
        .io_ctx(|| format!("unpack {} into {}", archive.display(), into.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_dir_falls_back_when_path_is_long() {
        let m = Managed::new(Path::new("/short"), Path::new("/short/runtime"));
        assert_eq!(m.socket_dir().unwrap(), PathBuf::from("/short/run"));
        let long = PathBuf::from("/").join("x".repeat(120));
        let m = Managed::new(&long, &long.join("runtime"));
        let dir = m.socket_dir().unwrap();
        assert!(dir.to_string_lossy().starts_with("/tmp/wb-"));
        assert!(dir.join(".s.PGSQL.5432").as_os_str().len() <= MAX_SOCKET_PATH);
    }

    #[test]
    fn current_platform_is_pinned() {
        if let Ok(t) = target() {
            assert_eq!(archive_sha256(t).unwrap().len(), 64);
        }
    }
}
