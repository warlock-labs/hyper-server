use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// `Fuse` is a wrapper around a future that ensures it can only complete once.
/// After the wrapped future completes, all subsequent polls will return `Poll::Pending`.
///
/// This struct is borrowed from the `futures-util` crate and is used in the hyper server.
/// LICENSE: MIT or Apache-2.0
#[pin_project]
pub(crate) struct Fuse<F> {
    /// The wrapped future. Once it completes, this will be set to `None`.
    #[pin]
    pub(crate) inner: Option<F>,
}

impl<F> Future for Fuse<F>
where
    F: Future,
{
    /// The output type is the same as the wrapped future's output type.
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Match on the pinned inner future
        match self.as_mut().project().inner.as_pin_mut() {
            // If we have a future, poll it
            Some(fut) => fut.poll(cx).map(|output| {
                // If the future completes, set inner to None
                // This ensures that future calls to poll will return Poll::Pending
                self.project().inner.set(None);
                output
            }),
            // If inner is None, it means the future has already completed
            // So we return Poll::Pending
            None => Poll::Pending,
        }
    }
}
