use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

// From `futures-util` crate, borrowed since this is the only dependency hyper-server requires.
// LICENSE: MIT or Apache-2.0
// A future which only yields `Poll::Ready` once, and thereafter yields `Poll::Pending`.
#[pin_project]
pub(crate) struct Fuse<F> {
    #[pin]
    pub(crate) inner: Option<F>,
}

impl<F> Future for Fuse<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project().inner.as_pin_mut() {
            Some(fut) => fut.poll(cx).map(|output| {
                self.project().inner.set(None);
                output
            }),
            None => Poll::Pending,
        }
    }
}
