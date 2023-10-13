use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// A thread-safe utility that provides a one-time notification to waiters.
/// It utilizes an atomic boolean to ensure that the notification is sent only once.
#[derive(Debug, Default)]
pub(crate) struct NotifyOnce {
    /// A flag indicating whether a notification has been sent.
    notified: AtomicBool,
    /// An asynchronous primitive from the Tokio library used for notifying tasks.
    notify: Notify,
}

impl NotifyOnce {
    /// Notifies all waiting tasks, ensuring that the notification happens only once.
    pub(crate) fn notify_waiters(&self) {
        // Atomically set the `notified` flag to true.
        self.notified.store(true, Ordering::SeqCst);

        // Notify all waiting tasks.
        self.notify.notify_waiters();
    }

    /// Checks whether a notification has been sent.
    ///
    /// Returns:
    /// - `true` if the notification has already been sent.
    /// - `false` otherwise.
    pub(crate) fn is_notified(&self) -> bool {
        self.notified.load(Ordering::SeqCst)
    }

    /// Awaits until a notification has been sent.
    ///
    /// This asynchronous function will immediately complete if a notification
    /// has already been sent, otherwise it will await until it's notified.
    pub(crate) async fn notified(&self) {
        let future = self.notify.notified();

        // If not notified, await on the future.
        if !self.notified.load(Ordering::SeqCst) {
            future.await;
        }
    }
}