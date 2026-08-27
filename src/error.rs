//! Errors returned by goatd operations.

use std::fmt;

/// An invalid input, malformed PACE file, invalid decomposition, oversized
/// problem, or failed FlowCutter construction.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Invalid arguments to an algorithm; the sentence names the violated
    /// input contract.
    InvalidInput(String),
    /// PACE text that does not describe a graph or a decomposition; the
    /// sentence names the line or id at fault.
    Parse(String),
    /// A tree decomposition that does not satisfy its structural or graph
    /// contract; the sentence names the first violation found.
    InvalidDecomposition(String),
    /// An input exceeds an algorithm's supported representation or allocation
    /// limit; the sentence names the limit.
    TooLarge(String),
    /// The FlowCutter builder returned no decomposition.
    NoDecomposition,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidInput(what)
            | Error::Parse(what)
            | Error::InvalidDecomposition(what)
            | Error::TooLarge(what) => f.write_str(what),
            Error::NoDecomposition => f.write_str("FlowCutter returned no decomposition"),
        }
    }
}

impl std::error::Error for Error {}
