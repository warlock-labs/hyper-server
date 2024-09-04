use std::future::Future;
use std::{fmt, io};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;
use hyper::server::accept::Accept;
use tokio::net::TcpListener;
use std::net::TcpListener as StdTcpListener;
use tokio::time::Sleep;
use socket2::TcpKeepalive;
use log::{debug, error, trace};
use crate::compat::AddrStream;
use crate::server::io_other;

#[derive(Default, Debug, Clone, Copy)]
struct TcpKeepaliveConfig {
    time: Option<Duration>,
    interval: Option<Duration>,
    retries: Option<u32>,
}

impl TcpKeepaliveConfig {
    /// Converts into a `socket2::TcpKeealive` if there is any keep alive configuration.
    fn into_socket2(self) -> Option<TcpKeepalive> {
        let mut dirty = false;
        let mut ka = TcpKeepalive::new();
        if let Some(time) = self.time {
            ka = ka.with_time(time);
            dirty = true
        }
        if let Some(interval) = self.interval {
            ka = Self::ka_with_interval(ka, interval, &mut dirty)
        };
        if let Some(retries) = self.retries {
            ka = Self::ka_with_retries(ka, retries, &mut dirty)
        };
        if dirty {
            Some(ka)
        } else {
            None
        }
    }

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "linux",
        target_os = "netbsd",
        target_vendor = "apple",
        windows,
    ))]
    fn ka_with_interval(ka: TcpKeepalive, interval: Duration, dirty: &mut bool) -> TcpKeepalive {
        *dirty = true;
        ka.with_interval(interval)
    }

    #[cfg(not(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "linux",
        target_os = "netbsd",
        target_vendor = "apple",
        windows,
    )))]
    fn ka_with_interval(ka: TcpKeepalive, _: Duration, _: &mut bool) -> TcpKeepalive {
        ka // no-op as keepalive interval is not supported on this platform
    }

    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "linux",
        target_os = "netbsd",
        target_vendor = "apple",
    ))]
    fn ka_with_retries(ka: TcpKeepalive, retries: u32, dirty: &mut bool) -> TcpKeepalive {
        *dirty = true;
        ka.with_retries(retries)
    }

    #[cfg(not(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "linux",
        target_os = "netbsd",
        target_vendor = "apple",
    )))]
    fn ka_with_retries(ka: TcpKeepalive, _: u32, _: &mut bool) -> TcpKeepalive {
        ka // no-op as keepalive retries is not supported on this platform
    }
}


/// A stream of connections from binding to an address.
#[must_use = "streams do nothing unless polled"]
pub struct AddrIncoming {
    addr: SocketAddr,
    listener: TcpListener,
    sleep_on_errors: bool,
    tcp_keepalive_config: TcpKeepaliveConfig,
    tcp_nodelay: bool,
    timeout: Option<Pin<Box<Sleep>>>,
}

impl AddrIncoming {
    pub(super) fn new(addr: &SocketAddr) -> Result<Self,std::io::Error> {
        let std_listener = StdTcpListener::bind(addr).map_err(io_other)?;

        AddrIncoming::from_std(std_listener)
    }

    pub(super) fn from_std(std_listener: StdTcpListener) -> Result<Self,std::io::Error>  {
        // TcpListener::from_std doesn't set O_NONBLOCK
        std_listener
            .set_nonblocking(true)
            .map_err(io_other)?;
        let listener = TcpListener::from_std(std_listener).map_err(io_other)?;
        AddrIncoming::from_listener(listener)
    }

    /// Creates a new `AddrIncoming` binding to provided socket address.
    pub fn bind(addr: &SocketAddr) -> Result<Self,std::io::Error> {
        AddrIncoming::new(addr)
    }

    /// Creates a new `AddrIncoming` from an existing `tokio::net::TcpListener`.
    pub fn from_listener(listener: TcpListener) -> Result<Self,std::io::Error> {
        let addr = listener.local_addr().map_err(io_other)?;
        Ok(AddrIncoming {
            listener,
            addr,
            sleep_on_errors: true,
            tcp_keepalive_config: TcpKeepaliveConfig::default(),
            tcp_nodelay: false,
            timeout: None,
        })
    }

    /// Get the local address bound to this listener.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Set the duration to remain idle before sending TCP keepalive probes.
    ///
    /// If `None` is specified, keepalive is disabled.
    pub fn set_keepalive(&mut self, time: Option<Duration>) -> &mut Self {
        self.tcp_keepalive_config.time = time;
        self
    }

    /// Set the duration between two successive TCP keepalive retransmissions,
    /// if acknowledgement to the previous keepalive transmission is not received.
    pub fn set_keepalive_interval(&mut self, interval: Option<Duration>) -> &mut Self {
        self.tcp_keepalive_config.interval = interval;
        self
    }

    /// Set the number of retransmissions to be carried out before declaring that remote end is not available.
    pub fn set_keepalive_retries(&mut self, retries: Option<u32>) -> &mut Self {
        self.tcp_keepalive_config.retries = retries;
        self
    }

    /// Set the value of `TCP_NODELAY` option for accepted connections.
    pub fn set_nodelay(&mut self, enabled: bool) -> &mut Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Set whether to sleep on accept errors.
    ///
    /// A possible scenario is that the process has hit the max open files
    /// allowed, and so trying to accept a new connection will fail with
    /// `EMFILE`. In some cases, it's preferable to just wait for some time, if
    /// the application will likely close some files (or connections), and try
    /// to accept the connection again. If this option is `true`, the error
    /// will be logged at the `error` level, since it is still a big deal,
    /// and then the listener will sleep for 1 second.
    ///
    /// In other cases, hitting the max open files should be treat similarly
    /// to being out-of-memory, and simply error (and shutdown). Setting
    /// this option to `false` will allow that.
    ///
    /// Default is `true`.
    pub fn set_sleep_on_errors(&mut self, val: bool) {
        self.sleep_on_errors = val;
    }

    fn poll_next_(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<AddrStream>> {
        // Check if a previous timeout is active that was set by IO errors.
        if let Some(ref mut to) = self.timeout {
            ready!(Pin::new(to).poll(cx));
        }
        self.timeout = None;

        loop {
            match ready!(self.listener.poll_accept(cx)) {
                Ok((socket, remote_addr)) => {
                    if let Some(tcp_keepalive) = &self.tcp_keepalive_config.into_socket2() {
                        let sock_ref = socket2::SockRef::from(&socket);
                        if let Err(e) = sock_ref.set_tcp_keepalive(tcp_keepalive) {
                            trace!("error trying to set TCP keepalive: {}", e);
                        }
                    }
                    if let Err(e) = socket.set_nodelay(self.tcp_nodelay) {
                        trace!("error trying to set TCP nodelay: {}", e);
                    }
                    let local_addr = socket.local_addr()?;
                    return Poll::Ready(Ok(AddrStream::new(socket, remote_addr, local_addr)));
                }
                Err(e) => {
                    // Connection errors can be ignored directly, continue by
                    // accepting the next request.
                    if is_connection_error(&e) {
                        debug!("accepted connection already errored: {}", e);
                        continue;
                    }

                    if self.sleep_on_errors {
                        error!("accept error: {}", e);

                        // Sleep 1s.
                        let mut timeout = Box::pin(tokio::time::sleep(Duration::from_secs(1)));

                        match timeout.as_mut().poll(cx) {
                            Poll::Ready(()) => {
                                // Wow, it's been a second already? Ok then...
                                continue;
                            }
                            Poll::Pending => {
                                self.timeout = Some(timeout);
                                return Poll::Pending;
                            }
                        }
                    } else {
                        return Poll::Ready(Err(e));
                    }
                }
            }
        }
    }
}

impl Accept for AddrIncoming {
    type Conn = AddrStream;
    type Error = io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let result = ready!(self.poll_next_(cx));
        Poll::Ready(Some(result))
    }
}

/// This function defines errors that are per-connection. Which basically
/// means that if we get this error from `accept()` system call it means
/// next connection might be ready to be accepted.
///
/// All other errors will incur a timeout before next `accept()` is performed.
/// The timeout is useful to handle resource exhaustion errors like ENFILE
/// and EMFILE. Otherwise, could enter into tight loop.
fn is_connection_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
    )
}

impl fmt::Debug for AddrIncoming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AddrIncoming")
            .field("addr", &self.addr)
            .field("sleep_on_errors", &self.sleep_on_errors)
            .field("tcp_keepalive_config", &self.tcp_keepalive_config)
            .field("tcp_nodelay", &self.tcp_nodelay)
            .finish()
    }
}
