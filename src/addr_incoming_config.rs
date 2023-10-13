use std::time::Duration;

/// Configuration settings for the `AddrIncoming`.
///
/// This configuration structure is designed to be used in conjunction with the
/// [`AddrIncoming`](hyper::server::conn::AddrIncoming) type from the Hyper crate.
/// It provides a mechanism to customize server settings like TCP keepalive probes,
/// error handling, and other TCP socket-level configurations.
#[derive(Debug, Clone)]
pub struct AddrIncomingConfig {
    pub(crate) tcp_sleep_on_accept_errors: bool,
    pub(crate) tcp_keepalive: Option<Duration>,
    pub(crate) tcp_keepalive_interval: Option<Duration>,
    pub(crate) tcp_keepalive_retries: Option<u32>,
    pub(crate) tcp_nodelay: bool,
}

impl Default for AddrIncomingConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl AddrIncomingConfig {
    /// Creates a new `AddrIncomingConfig` with default settings.
    ///
    /// # Default Settings
    /// - Sleep on accept errors: `true`
    /// - TCP keepalive probes: Disabled (`None`)
    /// - Duration between keepalive retransmissions: None
    /// - Number of keepalive retransmissions: None
    /// - `TCP_NODELAY` option: `false`
    ///
    /// # Returns
    ///
    /// A new `AddrIncomingConfig` instance with default settings.
    pub fn new() -> AddrIncomingConfig {
        Self {
            tcp_sleep_on_accept_errors: true,
            tcp_keepalive: None,
            tcp_keepalive_interval: None,
            tcp_keepalive_retries: None,
            tcp_nodelay: false,
        }
    }

    /// Creates a cloned copy of the current configuration.
    ///
    /// This method can be useful when you want to preserve the original settings and
    /// create a modified configuration based on the current one.
    ///
    /// # Returns
    ///
    /// A cloned `AddrIncomingConfig`.
    pub fn build(&mut self) -> Self {
        self.clone()
    }

    /// Specifies whether to pause (sleep) when an error occurs while accepting a connection.
    ///
    /// This can be useful to prevent rapidly exhausting file descriptors in scenarios
    /// where errors might be transient or frequent.
    ///
    /// # Parameters
    ///
    /// - `val`: Whether to sleep on accept errors. Default is `true`.
    ///
    /// # Returns
    ///
    /// A mutable reference to the current `AddrIncomingConfig`.
    pub fn tcp_sleep_on_accept_errors(&mut self, val: bool) -> &mut Self {
        self.tcp_sleep_on_accept_errors = val;
        self
    }

    /// Configures the frequency of TCP keepalive probes.
    ///
    /// TCP keepalive probes are used to detect whether a peer is still connected.
    ///
    /// # Parameters
    ///
    /// - `val`: Duration between keepalive probes. Setting to `None` disables keepalive probes. Default is `None`.
    ///
    /// # Returns
    ///
    /// A mutable reference to the current `AddrIncomingConfig`.
    pub fn tcp_keepalive(&mut self, val: Option<Duration>) -> &mut Self {
        self.tcp_keepalive = val;
        self
    }

    /// Configures the duration between two successive TCP keepalive retransmissions.
    ///
    /// If an acknowledgment to a previous keepalive probe isn't received within this duration,
    /// a new probe will be sent.
    ///
    /// # Parameters
    ///
    /// - `val`: Duration between keepalive retransmissions. Default is no interval (`None`).
    ///
    /// # Returns
    ///
    /// A mutable reference to the current `AddrIncomingConfig`.
    pub fn tcp_keepalive_interval(&mut self, val: Option<Duration>) -> &mut Self {
        self.tcp_keepalive_interval = val;
        self
    }

    /// Configures the number of times to retransmit a TCP keepalive probe if no acknowledgment is received.
    ///
    /// After the specified number of retransmissions, the remote end is considered unavailable.
    ///
    /// # Parameters
    ///
    /// - `val`: Number of retransmissions before considering the remote end unavailable. Default is no retry (`None`).
    ///
    /// # Returns
    ///
    /// A mutable reference to the current `AddrIncomingConfig`.
    pub fn tcp_keepalive_retries(&mut self, val: Option<u32>) -> &mut Self {
        self.tcp_keepalive_retries = val;
        self
    }

    /// Configures the `TCP_NODELAY` option for accepted connections.
    ///
    /// When enabled, this option disables Nagle's algorithm, which can reduce latencies for small packets.
    ///
    /// # Parameters
    ///
    /// - `val`: Whether to enable `TCP_NODELAY`. Default is `false`.
    ///
    /// # Returns
    ///
    /// A mutable reference to the current `AddrIncomingConfig`.
    pub fn tcp_nodelay(&mut self, val: bool) -> &mut Self {
        self.tcp_nodelay = val;
        self
    }
}