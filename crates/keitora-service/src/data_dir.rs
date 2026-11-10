// SPDX-License-Identifier: MIT OR Apache-2.0

//! The folder where the service keeps its files: `%ProgramData%\Keitora`,
//! or the one given with `--data-dir` in console mode.
//!
//! Who may write there is decided by its access control list, which the
//! installer and the service set up later (T-5.3, T-6.1); until then, a
//! standard user who creates the folder before Keitora is installed can
//! write to it (`docs/threat-model.md`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The name of the service's folder inside `%ProgramData%`.
const FOLDER_NAME: &str = "Keitora";

/// The name of the folder of log files inside the data folder.
const LOGS_FOLDER_NAME: &str = "logs";

/// Why the default data folder can't be found.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DataDirError {
    /// The `ProgramData` environment variable isn't set.
    #[error("the ProgramData environment variable isn't set")]
    NotSet,
    /// The `ProgramData` environment variable isn't an absolute path, so the
    /// folder would depend on the working directory.
    #[error("the ProgramData environment variable isn't an absolute path")]
    NotAbsolute,
}

/// `%ProgramData%\Keitora`, the service's data folder, read from the
/// `ProgramData` environment variable.
///
/// A service's environment comes from the system's, which only an
/// administrator can change.
///
/// # Errors
///
/// Returns a [`DataDirError`] if the variable isn't set or isn't an absolute
/// path.
pub fn default_data_dir() -> Result<PathBuf, DataDirError> {
    data_dir_in(std::env::var_os("ProgramData"))
}

/// `<program_data>\Keitora`, given the value of `ProgramData`.
fn data_dir_in(program_data: Option<OsString>) -> Result<PathBuf, DataDirError> {
    let program_data = PathBuf::from(program_data.ok_or(DataDirError::NotSet)?);
    if !program_data.is_absolute() {
        return Err(DataDirError::NotAbsolute);
    }
    Ok(program_data.join(FOLDER_NAME))
}

/// The folder of log files inside `data_dir`: `<data_dir>\logs`.
#[must_use]
pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(LOGS_FOLDER_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_folder_is_keitora_inside_program_data() {
        assert_eq!(
            data_dir_in(Some(r"C:\ProgramData".into())),
            Ok(PathBuf::from(r"C:\ProgramData\Keitora"))
        );
    }

    #[test]
    fn a_missing_or_relative_program_data_is_an_error() {
        assert_eq!(data_dir_in(None), Err(DataDirError::NotSet));
        assert_eq!(data_dir_in(Some("".into())), Err(DataDirError::NotAbsolute));
        assert_eq!(
            data_dir_in(Some("ProgramData".into())),
            Err(DataDirError::NotAbsolute)
        );
        assert_eq!(
            data_dir_in(Some(r"\ProgramData".into())),
            Err(DataDirError::NotAbsolute)
        );
    }

    #[test]
    fn logs_go_in_a_logs_folder() {
        assert_eq!(
            logs_dir(Path::new(r"C:\ProgramData\Keitora")),
            PathBuf::from(r"C:\ProgramData\Keitora\logs")
        );
    }
}
