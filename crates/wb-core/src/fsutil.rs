//! Small file-system helpers: atomic writes, private files, cross-process locks,
//! checksum-verified downloads.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{ErrorKind, IoContext, Result, err};

/// Writes a file atomically (temporary file in the same directory, then rename).
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).io_ctx(|| format!("create {}", dir.display()))?;
    let tmp = tmp_sibling(path);
    {
        let mut f = File::create(&tmp).io_ctx(|| format!("create {}", tmp.display()))?;
        f.write_all(data)
            .io_ctx(|| format!("write {}", tmp.display()))?;
        f.sync_all().io_ctx(|| format!("sync {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).io_ctx(|| format!("rename to {}", path.display()))
}

/// Writes a file readable only by the current user (0600 on Unix), atomically.
pub fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    ensure_private_dir(dir)?;
    let tmp = tmp_sibling(path);
    {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .io_ctx(|| format!("create {}", tmp.display()))?;
        f.write_all(data)
            .io_ctx(|| format!("write {}", tmp.display()))?;
        f.sync_all().io_ctx(|| format!("sync {}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).io_ctx(|| format!("rename to {}", path.display()))
}

/// Creates a directory (and parents) readable only by the current user (0700 on Unix).
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).io_ctx(|| format!("create {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .io_ctx(|| format!("chmod {}", dir.display()))?;
    }
    Ok(())
}

fn tmp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp-{}", std::process::id()))
}

/// An exclusive advisory lock on a file, released on drop.
pub struct FileLock {
    file: File,
    path: PathBuf,
}

impl FileLock {
    /// Blocks until the lock is acquired.
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).io_ctx(|| format!("create {}", dir.display()))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .io_ctx(|| format!("open lock {}", path.display()))?;
        file.lock().io_ctx(|| format!("lock {}", path.display()))?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Downloads `url` to `dest` unless a file with the right checksum is already there.
pub fn download_verified(
    url: &str,
    dest: &Path,
    sha256: &str,
    progress: &dyn Fn(&str),
) -> Result<()> {
    if dest.is_file() && file_sha256(dest)? == sha256 {
        return Ok(());
    }
    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")))?;
    progress(&format!("Downloading {url}"));
    let resp = ureq::get(url)
        .call()
        .map_err(|e| err(ErrorKind::DownloadFailed, format!("{url}: {e}")))?;
    let mut reader = resp.into_body().into_reader();
    let partial = dest.with_extension("partial");
    let mut file = std::fs::File::create(&partial)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| err(ErrorKind::DownloadFailed, format!("{url}: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.sync_all()?;
    let got = hex::encode(hasher.finalize());
    if got != sha256 {
        let _ = std::fs::remove_file(&partial);
        return Err(err(
            ErrorKind::ChecksumMismatch,
            format!("{url}: expected sha256 {sha256}, got {got}"),
        ));
    }
    std::fs::rename(&partial, dest)?;
    Ok(())
}

/// Hex SHA-256 of a file.
pub fn file_sha256(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Free space in bytes on the file system holding `path` (best effort).
pub fn available_space(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    fs_available(existing)
}

#[cfg(unix)]
fn fs_available(path: &Path) -> Option<u64> {
    // `df -Pk` is available on Linux and macOS and avoids unsafe FFI.
    let out = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1)?;
    let avail_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(avail_kb * 1024)
}

#[cfg(not(unix))]
fn fs_available(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_file_mode() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("secrets/db.toml");
        write_private(&p, b"x").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(p.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn lock_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let l = FileLock::acquire(&d.path().join("x.lock")).unwrap();
        drop(l);
        FileLock::acquire(&d.path().join("x.lock")).unwrap();
        assert!(available_space(d.path()).unwrap_or(1) > 0);
    }
}
