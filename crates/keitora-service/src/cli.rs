// SPDX-License-Identifier: MIT OR Apache-2.0

//! The command line of the `keitora-service` binary.
//!
//! ```text
//! keitora-service                               service mode (started by the SCM)
//! keitora-service --console [--data-dir <DIR>]  console mode (development builds only)
//! ```
//!
//! Anything else is rejected before anything is written to disk. Console
//! mode exists only in builds with debug assertions (the `dev` and `test`
//! profiles): a release build rejects `--console` like any unknown argument,
//! so the installed service can't be started in a mode meant for development
//! (F0 security review, L9).

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// The option that selects console mode.
const CONSOLE: &str = "--console";
/// The option that moves the data folder; it takes a value.
const DATA_DIR: &str = "--data-dir";

/// Whether this build has console mode: only builds with debug assertions.
pub const CONSOLE_MODE_AVAILABLE: bool = cfg!(debug_assertions);

/// How the binary runs, as chosen by its command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// No arguments: hand control to the Service Control Manager.
    Service,
    /// `--console`: run in the foreground until stopped.
    Console {
        /// The folder given with `--data-dir`, if any, instead of
        /// `%ProgramData%\Keitora`.
        data_dir: Option<PathBuf>,
    },
}

/// Why a command line was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    /// An argument that isn't an option of this build, or a positional
    /// argument.
    #[error("unknown argument `{0}`")]
    Unknown(String),
    /// An option given more than once.
    #[error("`{0}` is given more than once")]
    Repeated(&'static str),
    /// An option that needs a value came last, or with an empty value.
    #[error("`{0}` needs a value")]
    MissingValue(&'static str),
    /// `--data-dir` without `--console`: the service always uses
    /// `%ProgramData%\Keitora`.
    #[error("`{DATA_DIR}` can only be used together with `{CONSOLE}`")]
    DataDirWithoutConsole,
}

/// How to use the binary, for the message that follows a [`CliError`].
#[must_use]
pub fn usage(console_mode_available: bool) -> String {
    let mut usage = String::from(
        "usage:\n  keitora-service                               \
         run as a Windows service (started by the Service Control Manager)",
    );
    if console_mode_available {
        usage.push_str(
            "\n  keitora-service --console [--data-dir <DIR>]  \
             run in this terminal until stdin closes or Ctrl+C",
        );
    }
    usage
}

/// Reads the command line `args`, without the program name.
///
/// `console_mode_available` says whether `--console` and `--data-dir` are
/// options at all; the binary passes [`CONSOLE_MODE_AVAILABLE`]. When it is
/// `false`, they are rejected as unknown arguments.
///
/// # Errors
///
/// Returns a [`CliError`] for an unknown option, a positional argument,
/// `--data-dir` without `--console`, `--data-dir` without a value, and an
/// option given twice.
pub fn parse<I>(args: I, console_mode_available: bool) -> Result<Mode, CliError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut console = false;
    let mut data_dir: Option<PathBuf> = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if console_mode_available && arg == OsStr::new(CONSOLE) {
            if console {
                return Err(CliError::Repeated(CONSOLE));
            }
            console = true;
        } else if console_mode_available && arg == OsStr::new(DATA_DIR) {
            if data_dir.is_some() {
                return Err(CliError::Repeated(DATA_DIR));
            }
            let value = args
                .next()
                .filter(|value| !value.is_empty())
                .ok_or(CliError::MissingValue(DATA_DIR))?;
            data_dir = Some(PathBuf::from(value));
        } else {
            return Err(CliError::Unknown(arg.to_string_lossy().into_owned()));
        }
    }
    match (console, data_dir) {
        (true, data_dir) => Ok(Mode::Console { data_dir }),
        (false, None) => Ok(Mode::Service),
        (false, Some(_)) => Err(CliError::DataDirWithoutConsole),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str], console_mode_available: bool) -> Result<Mode, CliError> {
        parse(args.iter().map(OsString::from), console_mode_available)
    }

    fn console(data_dir: Option<&str>) -> Mode {
        Mode::Console {
            data_dir: data_dir.map(PathBuf::from),
        }
    }

    #[test]
    fn no_arguments_is_service_mode_in_every_build() {
        assert_eq!(parse_strs(&[], true), Ok(Mode::Service));
        assert_eq!(parse_strs(&[], false), Ok(Mode::Service));
    }

    #[test]
    fn console_mode_takes_an_optional_data_dir_in_either_order() {
        assert_eq!(parse_strs(&["--console"], true), Ok(console(None)));
        assert_eq!(
            parse_strs(&["--console", "--data-dir", r"C:\dev\data"], true),
            Ok(console(Some(r"C:\dev\data")))
        );
        assert_eq!(
            parse_strs(&["--data-dir", "rel dir", "--console"], true),
            Ok(console(Some("rel dir")))
        );
    }

    #[test]
    fn data_dir_takes_the_next_argument_as_its_value_whatever_it_looks_like() {
        // A value that looks like an option is still the value; the
        // `--console` it swallowed is then missing.
        assert_eq!(
            parse_strs(&["--data-dir", "--console"], true),
            Err(CliError::DataDirWithoutConsole)
        );
        assert_eq!(
            parse_strs(&["--console", "--data-dir", "--console"], true),
            Ok(console(Some("--console")))
        );
    }

    #[test]
    fn invalid_command_lines_are_rejected() {
        let cases: [(&[&str], CliError); 9] = [
            (&["--bogus"], CliError::Unknown("--bogus".into())),
            (&["run"], CliError::Unknown("run".into())),
            (&["--console", "extra"], CliError::Unknown("extra".into())),
            (
                &["--console=yes"],
                CliError::Unknown("--console=yes".into()),
            ),
            (
                &["--data-dir=x", "--console"],
                CliError::Unknown("--data-dir=x".into()),
            ),
            (&["--data-dir", "x"], CliError::DataDirWithoutConsole),
            (
                &["--console", "--data-dir"],
                CliError::MissingValue("--data-dir"),
            ),
            (
                &["--console", "--data-dir", ""],
                CliError::MissingValue("--data-dir"),
            ),
            (
                &["--console", "--data-dir", "a", "--data-dir", "b"],
                CliError::Repeated("--data-dir"),
            ),
        ];
        for (args, expected) in cases {
            assert_eq!(parse_strs(args, true), Err(expected), "{args:?}");
        }
        assert_eq!(
            parse_strs(&["--console", "--console"], true),
            Err(CliError::Repeated("--console"))
        );
    }

    #[test]
    fn without_console_mode_its_options_are_unknown_arguments() {
        assert_eq!(
            parse_strs(&["--console"], false),
            Err(CliError::Unknown("--console".into()))
        );
        assert_eq!(
            parse_strs(&["--data-dir", "x", "--console"], false),
            Err(CliError::Unknown("--data-dir".into()))
        );
    }

    #[test]
    fn usage_mentions_console_mode_only_where_it_exists() {
        assert!(usage(true).contains("--console"));
        assert!(!usage(false).contains("--console"));
    }
}
