//! The one error type this crate reports.

use std::fmt;

/// What can go wrong reading a PACE file or asking the FlowCutter builder for a
/// decomposition. Every other entry point returns a decomposition
/// unconditionally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// PACE text that does not describe a graph or a decomposition; the
    /// sentence names the line or id at fault.
    Parse(String),
    /// A graph the FlowCutter builder cannot be handed at all: the sentence
    /// names the size that is over its limit.
    TooLarge(String),
    /// The FlowCutter builder returned no decomposition.
    NoDecomposition,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(what) | Error::TooLarge(what) => f.write_str(what),
            Error::NoDecomposition => f.write_str("FlowCutter returned no decomposition"),
        }
    }
}

impl std::error::Error for Error {}
