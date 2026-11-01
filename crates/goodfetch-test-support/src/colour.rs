//! The CLI's colour properties (ADR 0073), as helper cases any command can
//! run on a pseudo-terminal: its coloured output with the escape sequences
//! stripped equals its `NO_COLOR=1` output, so colour never carries
//! information alone; an empty `NO_COLOR` does not turn colour off; and
//! `--color always` turns it on over `NO_COLOR`.

use std::ffi::OsStr;

use crate::env::TestEnv;
use crate::pty::{PtyError, PtyOutput, pty};

/// One of the CLI's colour properties, which [`ColourProperty::check`]
/// runs on one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourProperty {
    /// The coloured output, stripped of its escape sequences, equals the
    /// `NO_COLOR=1` output, exit code included; a coloured run that shows
    /// no colour fails it, since its stripping would prove nothing.
    StrippedColourEqualsNoColor,
    /// `NO_COLOR` set to the empty string leaves colour on (no-color.org).
    EmptyNoColorKeepsColour,
    /// `--color always`, added after the command's arguments, turns colour
    /// on although `NO_COLOR=1`.
    ColorAlwaysOverridesNoColor,
}

impl ColourProperty {
    /// Runs the command, `executable` with `arguments`, on a pseudo-terminal
    /// as the property needs, and fails when it breaks the property.
    pub fn check(
        self,
        env: &TestEnv,
        executable: impl AsRef<OsStr>,
        arguments: &[&str],
    ) -> Result<(), ColourError> {
        let executable = executable.as_ref();
        let run = |no_color: Option<&str>, extra: &[&str]| -> Result<PtyOutput, PtyError> {
            let terminal = pty(env, executable).args(arguments).args(extra);
            match no_color {
                Some(value) => terminal.env("NO_COLOR", value),
                None => terminal,
            }
            .run()
        };
        match self {
            Self::StrippedColourEqualsNoColor => {
                let coloured = run(None, &[])?;
                if !coloured.has_colour() {
                    return Err(ColourError::NoColourShown { coloured });
                }
                let no_color = run(Some("1"), &[])?;
                if (coloured.exit, coloured.stripped()) == (no_color.exit, no_color.stripped()) {
                    Ok(())
                } else {
                    Err(ColourError::ColourCarriesInformation { coloured, no_color })
                }
            }
            Self::EmptyNoColorKeepsColour => {
                let output = run(Some(""), &[])?;
                if output.has_colour() {
                    Ok(())
                } else {
                    Err(ColourError::EmptyNoColorTurnsColourOff { output })
                }
            }
            Self::ColorAlwaysOverridesNoColor => {
                let output = run(Some("1"), &["--color", "always"])?;
                if output.has_colour() {
                    Ok(())
                } else {
                    Err(ColourError::ColorAlwaysIgnored { output })
                }
            }
        }
    }
}

/// How a command breaks a colour property.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ColourError {
    /// The coloured output, stripped, differs from the `NO_COLOR=1` output:
    /// colour carries information alone.
    #[error(
        "the coloured output, stripped, differs from the NO_COLOR=1 output:\n{coloured}\n\n{no_color}"
    )]
    ColourCarriesInformation {
        /// The run with colour.
        coloured: PtyOutput,
        /// The run with `NO_COLOR=1`.
        no_color: PtyOutput,
    },
    /// The run with colour on showed none, so its stripping proves
    /// nothing: the command never colours, or its terminal detection broke.
    #[error("the run with colour on showed no colour, so stripping it proves nothing:\n{coloured}")]
    NoColourShown {
        /// The run with colour on.
        coloured: PtyOutput,
    },
    /// An empty `NO_COLOR` turned colour off.
    #[error("an empty NO_COLOR turned colour off:\n{output}")]
    EmptyNoColorTurnsColourOff {
        /// The run with `NO_COLOR` empty.
        output: PtyOutput,
    },
    /// `--color always` left colour off under `NO_COLOR=1`.
    #[error("--color always left colour off under NO_COLOR=1:\n{output}")]
    ColorAlwaysIgnored {
        /// The run with `--color always`.
        output: PtyOutput,
    },
    /// A run on the pseudo-terminal failed.
    #[error(transparent)]
    Pty(#[from] PtyError),
}
