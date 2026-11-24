use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use reqwest::header::{ETAG, LAST_MODIFIED};
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::sync::Mutex;

use super::archive;
use super::install_manager::{Build, InstallManager};

const HOST: &str = "https://soloist-builds.spotifycdn.com";
pub const LIFETIME: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const BINARY: &str = "soloist";

static INSTALLING: Mutex<()> = Mutex::const_new(());

static BLOCK_DOWNLOADS: AtomicBool = AtomicBool::new(false);

pub fn block_downloads(blocked: bool) {
    BLOCK_DOWNLOADS.store(blocked, Ordering::Relaxed);
}

fn blocked() -> Result<(), InstallError> {
    if BLOCK_DOWNLOADS.load(Ordering::Relaxed) {
        return Err(InstallError::Network(
            "downloads are blocked by the dev tools".into(),
        ));
    }
    Ok(())
}

fn archive_for_cpu() -> Option<(&'static str, u16)> {
    match std::env::consts::ARCH {
        "x86_64" => Some(("soloist_release_x86_64.tar.gz", 62)),
        "aarch64" => Some(("soloist_release_arm64.tar.gz", 183)),
        _ => None,
    }
}

#[derive(Debug)]
pub enum InstallError {
    Unsupported,
    Network(String),
    Bad(String),
    Io(io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "there's no version for this computer's processor"),
            Self::Network(why) => write!(f, "couldn't download it: {why}"),
            Self::Bad(why) => write!(f, "the download didn't work: {why}"),
            Self::Io(e) => write!(f, "couldn't install it: {e}"),
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug)]
struct Published {
    etag: Option<String>,
    last_modified: Option<String>,
}

impl Published {
    fn is(&self, build: &Build) -> bool {
        match &self.etag {
            Some(_) => build.etag == self.etag,
            None => self.last_modified.is_some() && build.last_modified == self.last_modified,
        }
    }
}

fn url() -> Result<String, InstallError> {
    let (name, _) = archive_for_cpu().ok_or(InstallError::Unsupported)?;
    Ok(format!("{HOST}/{name}"))
}

fn published(response: &reqwest::Response) -> Published {
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    Published {
        etag: header(ETAG),
        last_modified: header(LAST_MODIFIED),
    }
}

pub async fn ensure(installs: &InstallManager, client: &Client) -> Result<Build, InstallError> {
    let _installing = INSTALLING.lock().await;
    if let Some(build) = installs.current() {
        if installs.binary(&build).is_file() {
            return Ok(build);
        }
        warn!(
            "Soloist's build {} is missing; installing it again",
            build.id
        );
    }
    let build = download(installs, client).await?;
    installs.switch_to(&build.id)?;
    Ok(build)
}

pub async fn update(
    installs: &InstallManager,
    client: &Client,
) -> Result<Option<Build>, InstallError> {
    let _installing = INSTALLING.lock().await;
    blocked()?;
    let response = client
        .head(url()?)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| InstallError::Network(e.to_string()))?;
    let published = published(&response);
    let record = installs.record();
    let current = installs.current();
    let same = current.as_ref().is_some_and(|build| published.is(build));
    let refused = published
        .etag
        .as_ref()
        .is_some_and(|etag| record.refused.contains(etag));
    if same || refused {
        return Ok(None);
    }
    let downloaded = record
        .builds
        .iter()
        .find(|build| published.is(build) && installs.binary(build).is_file());
    if let Some(build) = downloaded {
        info!(
            "The newer Soloist build {} is downloaded already",
            build.version
        );
        return Ok(Some(build.clone()));
    }
    info!("A newer Soloist build is published ({published:?})");
    download(installs, client).await.map(Some)
}

async fn download(installs: &InstallManager, client: &Client) -> Result<Build, InstallError> {
    blocked()?;
    let url = url()?;
    info!("Downloading Soloist from {url}");
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| InstallError::Network(e.to_string()))?;
    if response.status() != StatusCode::OK {
        return Err(InstallError::Network(format!(
            "the server answered {}",
            response.status()
        )));
    }
    let published = published(&response);
    let archive = response
        .bytes()
        .await
        .map_err(|e| InstallError::Network(e.to_string()))?;
    let sha256 = format!("{:x}", Sha256::digest(&archive));
    let id = sha256[..16].to_string();
    let expires_at = published
        .last_modified
        .as_deref()
        .and_then(|date| httpdate::parse_http_date(date).ok())
        .map(|made| made + LIFETIME)
        .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
        .map(|at| at.as_secs());

    let installed = installs
        .record()
        .build(&id)
        .filter(|build| installs.binary(build).is_file())
        .cloned();
    if let Some(known) = installed {
        info!("Soloist {} is installed already", known.version);
        let build = Build {
            etag: published.etag,
            last_modified: published.last_modified,
            expires_at,
            ..known
        };
        installs.note(&build)?;
        return Ok(build);
    }

    let refuse = |why: String| {
        if let Some(etag) = &published.etag {
            let _ = installs.refuse(etag);
        }
        InstallError::Bad(why)
    };
    let staging = installs.staging()?;
    archive::unpack_tar_gz(&archive, &staging).map_err(|e| refuse(e.to_string()))?;
    let binary = find_binary(&staging).ok_or_else(|| refuse("it holds no player".into()))?;
    check_elf(&staging.join(&binary)).map_err(refuse)?;
    let version = version(&staging.join(&binary)).await.map_err(refuse)?;
    info!(
        "Downloaded Soloist {version} ({} bytes, sha256 {sha256})",
        archive.len()
    );

    let build = Build {
        id,
        binary,
        etag: published.etag,
        last_modified: published.last_modified,
        sha256,
        version,
        expires_at,
    };
    installs.add(&staging, build.clone())?;
    Ok(build)
}

fn find_binary(dir: &Path) -> Option<PathBuf> {
    let mut folders = vec![PathBuf::new()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(dir.join(&folder)).ok()?.flatten() {
            let path = folder.join(entry.file_name());
            let kind = entry.file_type().ok()?;
            if kind.is_dir() {
                folders.push(path);
            } else if kind.is_file() && entry.file_name() == BINARY {
                return Some(path);
            }
        }
    }
    None
}

fn check_elf(binary: &Path) -> Result<(), String> {
    let (_, machine) = archive_for_cpu().ok_or("no build for this processor")?;
    let header = std::fs::read(binary).map_err(|e| e.to_string())?;
    let header = header.get(..20).ok_or("it's too short to be a program")?;
    if &header[..4] != b"\x7fELF" {
        return Err("it isn't a program".into());
    }
    let found = u16::from_le_bytes([header[18], header[19]]);
    if found != machine {
        return Err(format!("it's built for another processor ({found})"));
    }
    Ok(())
}

async fn version(binary: &Path) -> Result<String, String> {
    let output = tokio::time::timeout(
        VERSION_TIMEOUT,
        Command::new(binary)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "--version didn't finish".to_string())?
    .map_err(|e| format!("it doesn't run: {e}"))?;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "--version failed ({}): {}",
            output.status,
            why.trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.lines().next().unwrap_or_default().trim().to_string())
}

pub fn days_left(build: &Build) -> Option<i64> {
    let now = (super::unix_ms() / 1000) as i64;
    Some((build.expires_at? as i64 - now) / (24 * 60 * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_binary_in_a_folder() {
        let dir = std::env::temp_dir().join(format!("riff-soloist-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("soloist-1.0/bin")).unwrap();
        std::fs::write(dir.join("soloist-1.0/CHANGELOG.md"), "").unwrap();
        std::fs::write(dir.join("soloist-1.0/bin/soloist"), "").unwrap();
        assert_eq!(
            find_binary(&dir),
            Some(PathBuf::from("soloist-1.0/bin/soloist"))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_programs_for_this_cpu_pass() {
        let dir = std::env::temp_dir().join(format!("riff-soloist-elf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("soloist");
        std::fs::write(&file, b"#!/bin/sh\n").unwrap();
        assert!(check_elf(&file).is_err());
        if let Some((_, machine)) = archive_for_cpu() {
            let mut header = b"\x7fELF".to_vec();
            header.resize(18, 0);
            header.extend(machine.to_le_bytes());
            std::fs::write(&file, &header).unwrap();
            assert!(check_elf(&file).is_ok());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
