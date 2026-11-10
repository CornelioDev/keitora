// SPDX-License-Identifier: MIT OR Apache-2.0

//! Logging: `tracing` records written to daily files in
//! `<data dir>\logs`, which is `%ProgramData%\Keitora\logs` in service mode.
//!
//! - A file is named after its day, in UTC: `keitora.2026-11-10.log`. A new
//!   file starts at midnight UTC; starting again on the same day appends to
//!   that day's file, and an earlier day's file is never written to again.
//! - At most [`MAX_LOG_FILES`] files are kept: when a new one is opened, the
//!   oldest ones are deleted, so the folder can't grow without limit.
//! - Records are written as they happen, with no buffer, so nothing is lost
//!   when the process stops.
//! - If the folder can't be created or a file can't be opened, records go to
//!   stderr instead, and the problem is reported there. The service keeps
//!   running: refusing to run would let anyone who can break the log folder
//!   switch off blocking and strict mode. In service mode stderr goes
//!   nowhere, so those records are lost.
//! - Until T-5.3 secures the data folder ([`crate::data_dir`]), any standard
//!   user can create files in the log folder: plant a day's file before the
//!   service opens it, hold an opportunistic lock on one so that opening or
//!   deleting it hangs, along with every thread that logs, or steer which
//!   files the retention deletes through their creation times. If T-5.3's
//!   checks of the folder fail, logs go to stderr instead.
//!
//! # What may be logged
//!
//! Logs must never hold blocklists, URLs, window titles or other user data,
//! and values are never logged with `{:?}`, which prints every field of a
//! type, present or added later. Log what happened and, where needed, a
//! value chosen on purpose and printed with `Display`
//! (`docs/architecture.md`, "Storage").

use std::path::Path;
use std::{fs, io};

use tracing::level_filters::LevelFilter;
use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};
use tracing_subscriber::fmt::MakeWriter;

use crate::{error_chain, print_error};

/// How every log file's name starts.
pub const LOG_FILE_PREFIX: &str = "keitora";

/// How every log file's name ends, after a dot.
pub const LOG_FILE_SUFFIX: &str = "log";

/// How many daily log files are kept, today's included: two weeks.
pub const MAX_LOG_FILES: usize = 14;

/// The most detailed level that is logged.
const MAX_LEVEL: LevelFilter = LevelFilter::INFO;

/// Sends this process's `tracing` records to daily files in `logs_dir`,
/// creating the folder if needed. If that fails, sends them to stderr and
/// logs why there.
///
/// Call it once, as soon as the process may write to disk; later calls
/// change nothing and print an error on stderr.
pub fn init(logs_dir: &Path) {
    match daily_files(logs_dir) {
        Ok(files) => install(files),
        Err(error) => {
            install(io::stderr);
            tracing::error!(
                "can't write log files to {}, logging to stderr instead: {}",
                logs_dir.display(),
                error_chain(&error)
            );
        }
    }
}

/// Sends this process's `tracing` records to stderr, when there is no
/// folder for log files, and logs `reason` there.
///
/// Call it instead of [`init`], once.
pub fn init_without_files(reason: &dyn std::error::Error) {
    install(io::stderr);
    tracing::error!(
        "no folder for log files, logging to stderr instead: {}",
        error_chain(reason)
    );
}

/// Why [`daily_files`] can't write to a log folder.
#[derive(Debug, thiserror::Error)]
pub enum LogFilesError {
    /// The folder, or one of its parents, can't be created; for example,
    /// because a file has its name.
    #[error("can't create the log folder")]
    CreateFolder(#[source] io::Error),
    /// Today's file can't be opened.
    #[error("can't open today's log file")]
    OpenFile(#[source] InitError),
}

/// The writer of daily log files in `logs_dir`, which it creates if needed,
/// with its parents. It opens today's file, and deletes the oldest files
/// beyond [`MAX_LOG_FILES`].
///
/// # Errors
///
/// Returns a [`LogFilesError`] if the folder can't be created or today's
/// file can't be opened.
pub fn daily_files(logs_dir: &Path) -> Result<RollingFileAppender, LogFilesError> {
    // Created here, not left to `build`, which looks for old files to delete
    // before it creates the folder, and prints an error on stderr when it
    // isn't there yet.
    fs::create_dir_all(logs_dir).map_err(LogFilesError::CreateFolder)?;
    // `build` returns an error where `tracing_appender::rolling::daily`
    // would panic.
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(logs_dir)
        .map_err(LogFilesError::OpenFile)
}

/// Makes a subscriber that writes formatted records to `writer` the global
/// default.
fn install<W>(writer: W)
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let installed = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_max_level(MAX_LEVEL)
        .try_init();
    if let Err(error) = installed {
        print_error(&format!(
            "couldn't set up logging: {}",
            error_chain(error.as_ref())
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// An empty folder for one test, in the system's temporary folder.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("keitora-service-unit-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("couldn't create the scratch folder");
        dir
    }

    #[test]
    fn daily_files_creates_the_folder_and_a_file_named_after_the_day() {
        let dir = scratch("daily-files");
        let logs = dir.join("missing").join("logs");

        let files = daily_files(&logs).expect("the log folder can be created");
        drop(files);

        let names: Vec<String> = fs::read_dir(&logs)
            .expect("the log folder exists")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        let name = &names[0];
        // keitora.YYYY-MM-DD.log
        assert!(
            name.starts_with("keitora.") && name.ends_with(".log"),
            "{name}"
        );
        assert_eq!(name.len(), "keitora.2026-11-10.log".len(), "{name}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn daily_files_fails_instead_of_panicking_when_the_folder_is_a_file() {
        let dir = scratch("logs-is-a-file");
        let logs = dir.join("logs");
        fs::write(&logs, b"not a folder").expect("couldn't plant the file");

        assert!(matches!(
            daily_files(&logs),
            Err(LogFilesError::CreateFolder(_))
        ));
        assert_eq!(
            fs::read(&logs).expect("the file is still there"),
            b"not a folder"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_files_beyond_the_limit_are_deleted() {
        let dir = scratch("retention");
        let logs = dir.join("logs");
        fs::create_dir_all(&logs).expect("couldn't create the log folder");
        for day in 1..=20 {
            fs::write(logs.join(format!("keitora.2020-01-{day:02}.log")), b"old\n")
                .expect("couldn't write an old file");
        }

        drop(daily_files(&logs).expect("the log folder is usable"));

        let count = fs::read_dir(&logs).expect("the log folder exists").count();
        assert_eq!(count, MAX_LOG_FILES);
        let _ = fs::remove_dir_all(&dir);
    }
}
