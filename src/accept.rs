//! Module `accept` provides utilities for asynchronously processing and modifying IO streams and services.
//!
//! The primary trait exposed by this module is [`Accept`], which allows for asynchronous transformations
//! of input streams and services. The module also provides a default implementation, [`DefaultAcceptor`],
//! that performs no modifications and directly passes through the input stream and service.

use std::{
    future::{Future, Ready},
    io,
};

/// An asynchronous trait for processing and modifying IO streams and services.
///
/// Implementations of this trait can be used to modify or transform the input stream and service before
/// further processing. For instance, this trait could be used to perform initial authentication, logging,
/// or other setup operations on new connections.
pub trait Accept<I, S> {
    /// The modified or transformed IO stream produced by `accept`.
    type Stream;

    /// The modified or transformed service produced by `accept`.
    type Service;

    /// The Future type that is returned by `accept`.
    type Future: Future<Output = io::Result<(Self::Stream, Self::Service)>>;

    /// Asynchronously process and possibly modify the given IO stream and service.
    ///
    /// # Parameters:
    /// * `stream`: The incoming IO stream, typically a connection.
    /// * `service`: The associated service with the stream.
    ///
    /// # Returns:
    /// A future resolving to the modified stream and service, or an error.
    fn accept(&self, stream: I, service: S) -> Self::Future;
}

/// A default implementation of the [`Accept`] trait that performs no modifications.
///
/// This is a no-op acceptor that simply passes the provided stream and service through without any transformations.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultAcceptor;

impl DefaultAcceptor {
    /// Create a new default acceptor instance.
    ///
    /// # Returns:
    /// An instance of [`DefaultAcceptor`].
    pub fn new() -> Self {
        Self
    }
}

impl<I, S> Accept<I, S> for DefaultAcceptor {
    type Stream = I;
    type Service = S;
    type Future = Ready<io::Result<(Self::Stream, Self::Service)>>;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        std::future::ready(Ok((stream, service)))
    }
}
