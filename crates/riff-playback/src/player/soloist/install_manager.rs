use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Build {
    pub id: String,
    pub binary: PathBuf,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub sha256: String,
    pub version: String,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub builds: Vec<Build>,
    #[serde(default)]
    pub refused: Vec<String>,
}

impl Record {
    pub fn build(&self, id: &str) -> Option<&Build> {
        self.builds.iter().find(|build| build.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct InstallManager {
    root: PathBuf,
}

impl InstallManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn builds(&self) -> PathBuf {
        self.root.join("builds")
    }

    fn record_file(&self) -> PathBuf {
        self.root.join("install.json")
    }

    pub fn record(&self) -> Record {
        fs::read(self.record_file())
            .ok()
            .and_then(|json| serde_json::from_slice(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, record: &Record) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let json = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
        let temp = self.root.join("install.json.new");
        fs::write(&temp, json)?;
        fs::rename(temp, self.record_file())
    }

    fn linked(&self, link: &str) -> Option<Build> {
        let target = fs::read_link(self.root.join(link)).ok()?;
        let id = target.file_name()?.to_str()?.to_string();
        self.record().build(&id).cloned()
    }

    pub fn current(&self) -> Option<Build> {
        self.linked("current")
    }

    pub fn previous(&self) -> Option<Build> {
        self.linked("previous")
    }

    pub fn binary(&self, build: &Build) -> PathBuf {
        self.builds().join(&build.id).join(&build.binary)
    }

    pub fn staging(&self) -> io::Result<PathBuf> {
        let dir = self.root.join("staging");
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn add(&self, staging: &Path, build: Build) -> io::Result<()> {
        fs::create_dir_all(self.builds())?;
        let dir = self.builds().join(&build.id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::rename(staging, &dir)?;
        let mut record = self.record();
        record.builds.retain(|known| known.id != build.id);
        record.builds.push(build);
        self.save(&record)
    }

    pub fn note(&self, build: &Build) -> io::Result<()> {
        let mut record = self.record();
        if let Some(known) = record.builds.iter_mut().find(|known| known.id == build.id) {
            *known = build.clone();
        }
        self.save(&record)
    }

    pub fn switch_to(&self, id: &str) -> io::Result<()> {
        let old = self.current().filter(|build| build.id != id);
        self.link("current", id)?;
        if let Some(old) = &old {
            self.link("previous", &old.id)?;
        }
        self.keep_only(&[Some(id.to_string()), old.map(|build| build.id)])
    }

    pub fn roll_back(&self) -> io::Result<bool> {
        let (Some(bad), Some(previous)) = (self.current(), self.previous()) else {
            return Ok(false);
        };
        self.link("current", &previous.id)?;
        let _ = fs::remove_file(self.root.join("previous"));
        let mut record = self.record();
        if let Some(etag) = bad.etag {
            record.refused.push(etag);
        }
        self.save(&record)?;
        self.keep_only(&[Some(previous.id)])?;
        Ok(true)
    }

    pub fn refuse(&self, etag: &str) -> io::Result<()> {
        let mut record = self.record();
        if !record.refused.iter().any(|known| known == etag) {
            record.refused.push(etag.to_string());
        }
        self.save(&record)
    }

    fn link(&self, name: &str, id: &str) -> io::Result<()> {
        let temp = self.root.join(format!("{name}.new"));
        let _ = fs::remove_file(&temp);
        std::os::unix::fs::symlink(Path::new("builds").join(id), &temp)?;
        fs::rename(temp, self.root.join(name))
    }

    fn keep_only(&self, ids: &[Option<String>]) -> io::Result<()> {
        let keep: Vec<&str> = ids.iter().flatten().map(String::as_str).collect();
        let mut record = self.record();
        record
            .builds
            .retain(|build| keep.contains(&build.id.as_str()));
        self.save(&record)?;
        for entry in fs::read_dir(self.builds())?.flatten() {
            let name = entry.file_name();
            if !keep.iter().any(|id| name == **id) {
                fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installs(name: &str) -> InstallManager {
        let root =
            std::env::temp_dir().join(format!("riff-engine-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        InstallManager::new(root)
    }

    fn add(installs: &InstallManager, id: &str) {
        let staging = installs.staging().unwrap();
        fs::write(staging.join("engine"), id).unwrap();
        installs
            .add(
                &staging,
                Build {
                    id: id.to_string(),
                    binary: "engine".into(),
                    etag: Some(format!("etag-{id}")),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    #[test]
    fn keeps_the_build_in_use_and_the_one_before() {
        let installs = installs("switch");
        assert!(installs.current().is_none());
        for id in ["a", "b", "c"] {
            add(&installs, id);
            installs.switch_to(id).unwrap();
        }
        assert_eq!(installs.current().unwrap().id, "c");
        assert_eq!(installs.previous().unwrap().id, "b");
        let binary = installs.binary(&installs.current().unwrap());
        assert_eq!(fs::read_to_string(binary).unwrap(), "c");
        assert!(!installs.root().join("builds/a").exists());
        assert_eq!(installs.record().builds.len(), 2);
        fs::remove_dir_all(installs.root()).unwrap();
    }

    #[test]
    fn rolling_back_refuses_the_bad_build() {
        let installs = installs("rollback");
        add(&installs, "a");
        installs.switch_to("a").unwrap();
        assert!(!installs.roll_back().unwrap());
        add(&installs, "b");
        installs.switch_to("b").unwrap();
        assert!(installs.roll_back().unwrap());
        assert_eq!(installs.current().unwrap().id, "a");
        assert!(installs.previous().is_none());
        assert_eq!(installs.record().refused, ["etag-b"]);
        assert!(!installs.root().join("builds/b").exists());
        fs::remove_dir_all(installs.root()).unwrap();
    }
}
