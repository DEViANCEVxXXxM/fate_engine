use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "windowed")]
pub fn state_dir() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let var = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support"))
    } else {
        var("XDG_STATE_HOME")
            .filter(|path| path.is_absolute())
            .or_else(|| var("HOME").map(|home| home.join(".local/state")))
    }?;
    Some(base.join("fate_engine").join("duel_lab"))
}

pub fn read_bounded(path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(bytes)
}

/// Replace a file only after its new contents have been written and synced.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    for _ in 0..32 {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let staged = parent.join(format!(".duel-{}-{id}.tmp", std::process::id()));
        let mut file = match File::create_new(&staged) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        let result = written.and_then(|()| fs::rename(&staged, path));
        if result.is_err() {
            let _ = fs::remove_file(&staged);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create temporary save file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_a_file_and_limits_read_size() {
        let directory = std::env::temp_dir().join(format!(
            "fate-duel-storage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("checkpoint");
        write_atomic(&path, b"original").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(read_bounded(&path, 3).unwrap(), b"new");
        assert_eq!(
            read_bounded(&path, 2).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
