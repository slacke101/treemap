use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalMode {
    RecycleBin,
    Permanent,
}

pub fn is_protected_path(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }

    #[cfg(windows)]
    {
        let protected = [
            "WINDIR",
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
            "APPDATA",
            "LOCALAPPDATA",
        ]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
        if std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .is_some_and(|profile| {
                path == profile || profile.parent().is_some_and(|users| path == users)
            })
        {
            return true;
        }
        return protected
            .iter()
            .any(|protected_path| path == protected_path || path.starts_with(protected_path));
    }

    #[cfg(not(windows))]
    {
        ["/usr", "/etc", "/bin", "/sbin", "/System", "/Applications"]
            .iter()
            .map(PathBuf::from)
            .any(|protected_path| path == protected_path || path.starts_with(protected_path))
    }
}

pub fn remove_path(path: &Path, mode: RemovalMode) -> Result<(), String> {
    if is_protected_path(path) {
        return Err("protected system or application paths cannot be removed".to_string());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("links cannot be removed from TreeMap".to_string());
    }

    match mode {
        RemovalMode::RecycleBin => trash::delete(path).map_err(|error| error.to_string()),
        RemovalMode::Permanent => if metadata.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
        .map_err(|error| error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn permanently_removes_a_file_in_a_temporary_directory() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("open-source-core-remove-{unique}"));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("temporary.txt");
        fs::write(&path, b"temporary test data").unwrap();

        remove_path(&path, RemovalMode::Permanent).unwrap();

        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
