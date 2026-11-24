use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};

pub fn unpack_tar_gz(archive: &[u8], dest: &Path) -> io::Result<()> {
    unpack_tar(GzDecoder::new(archive), dest)
}

fn unpack_tar(tar: impl Read, dest: &Path) -> io::Result<()> {
    let mut archive = Archive::new(tar);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = inside(dest, &entry.path()?)?;
        match entry.header().entry_type() {
            EntryType::Regular => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                entry.unpack(&path)?;
                set_mode(&path, entry.header().mode()?)?;
            }
            EntryType::Directory => fs::create_dir_all(&path)?,
            _ => {
                return Err(invalid(&format!(
                    "{:?} isn't a plain file or folder",
                    entry.path()?
                )))
            }
        }
    }
    Ok(())
}

fn inside(dest: &Path, name: &Path) -> io::Result<PathBuf> {
    let mut path = dest.to_path_buf();
    for component in name.components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            _ => return Err(invalid(&format!("{name:?} points outside the archive"))),
        }
    }
    if path == dest {
        return Err(invalid("an entry has no name"));
    }
    Ok(path)
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o755))
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: usize = 512;

    fn entry(name: &str, kind: u8, body: &[u8], mode: u32) -> Vec<u8> {
        let mut header = [0u8; BLOCK];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        header[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
        header[156] = kind;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        let mut out = header.to_vec();
        out.extend_from_slice(body);
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
        out
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("riff-archive-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unpacks_files_and_folders() {
        let dir = scratch("ok");
        let mut tar = entry("build/", b'5', b"", 0o755);
        tar.extend(entry("build/soloist", b'0', b"\x7fELF", 0o4777));
        tar.extend([0u8; BLOCK * 2]);
        unpack_tar(tar.as_slice(), &dir).unwrap();
        let binary = dir.join("build/soloist");
        assert_eq!(fs::read(&binary).unwrap(), b"\x7fELF");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&binary).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unpacks_gzipped_long_names() {
        let dir = scratch("long");
        let name = format!("{}/soloist", "a".repeat(120));
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_entry_type(EntryType::Regular);
        builder
            .append_data(&mut header, &name, &b"\x7fELF"[..])
            .unwrap();
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        io::Write::write_all(&mut gz, &tar).unwrap();
        unpack_tar_gz(&gz.finish().unwrap(), &dir).unwrap();
        assert_eq!(fs::read(dir.join(&name)).unwrap(), b"\x7fELF");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_what_escapes_or_isnt_plain() {
        let dir = scratch("bad");
        for (name, kind) in [("../evil", b'0'), ("/etc/evil", b'0'), ("link", b'2')] {
            let tar = entry(name, kind, b"x", 0o644);
            assert!(
                unpack_tar(tar.as_slice(), &dir).is_err(),
                "{name} was taken"
            );
        }
        assert!(!dir.parent().unwrap().join("evil").exists());
        assert!(!dir.join("etc").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
