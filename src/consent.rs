use std::fs;
use std::path::PathBuf;

pub const TERMS_VERSION: &str = "2026-09-30-v4";

pub fn terms_accepted() -> Result<bool, String> {
    match fs::read_to_string(terms_acceptance_path()?) {
        Ok(version) => Ok(version.trim() == TERMS_VERSION),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub fn record_terms_acceptance() -> Result<(), String> {
    let path = terms_acceptance_path()?;
    let directory = path
        .parent()
        .ok_or_else(|| "cannot determine the agreement data directory".to_string())?;
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    fs::write(path, TERMS_VERSION).map_err(|error| error.to_string())
}

fn terms_acceptance_path() -> Result<PathBuf, String> {
    let account = std::env::var("USERDOMAIN")
        .unwrap_or_default()
        .chars()
        .chain(std::iter::once('_'))
        .chain(
            std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "default".to_string())
                .chars(),
        )
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    Ok(open_source_core::cache::data_directory()?.join(format!("terms-{account}.accepted")))
}
