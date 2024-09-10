use std::{error::Error as StdError, fmt};

/// A type alias for the source of an error, which is a boxed trait object.
/// This allows for dynamic dispatch and type erasure of the original error type.
type Source = Box<dyn StdError + Send + Sync + 'static>;

/// Represents errors that originate from the server.
/// This struct provides a public API for error handling.
pub struct Error {
    inner: ErrorImpl,
}

/// The internal implementation of the Error struct.
/// This separation allows for better control over the public API.
struct ErrorImpl {
    kind: Kind,
    source: Option<Source>,
}

/// Enum representing different kinds of errors that can occur.
/// Currently, only includes a Transport variant, but can be extended for more error types.
#[derive(Debug)]
pub(crate) enum Kind {
    Transport,
}

impl Error {
    /// Creates a new Error with a specific kind.
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            inner: ErrorImpl { kind, source: None },
        }
    }

    /// Attaches a source error to this Error.
    /// This method consumes self and returns a new Error, allowing for method chaining.
    pub(crate) fn with(mut self, source: impl Into<Source>) -> Self {
        self.inner.source = Some(source.into());
        self
    }

    /// Creates a new Transport Error with the given source.
    /// This is a convenience method combining new() and with().
    pub(crate) fn from_source(source: impl Into<crate::Error>) -> Self {
        Error::new(Kind::Transport).with(source)
    }

    /// Returns a string slice describing the error.
    fn description(&self) -> &str {
        match &self.inner.kind {
            Kind::Transport => "transport error",
        }
    }
}

/// Implements the Debug trait for Error.
/// This provides a custom debug representation of the error.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_tuple("tonic::transport::Error");

        f.field(&self.inner.kind);

        if let Some(source) = &self.inner.source {
            f.field(source);
        }

        f.finish()
    }
}

/// Implements the Display trait for Error.
/// This provides a user-friendly string representation of the error.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// Implements the std::error::Error trait for Error.
/// This allows our custom Error to be used with the standard error handling mechanisms in Rust.
impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner
            .source
            .as_ref()
            .map(|source| &**source as &(dyn StdError + 'static))
    }
}
