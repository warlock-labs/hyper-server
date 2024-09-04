use hyper_util::server::conn::auto::Builder as HttpAutoConfig;
use std::time::Duration;
use hyper_util::rt::TokioExecutor;


/// Represents a configuration for the [`Http`] protocol.
/// This allows for detailed customization of various HTTP/1 and HTTP/2 settings.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// The HTTP configuration from the `hyper-util` crate.
    pub(crate) cfg: HttpAutoConfig<TokioExecutor>,
}

impl Default for HttpConfig {
    /// Provides a default HTTP configuration.
    fn default() -> Self {
        Self::new()
    }
}

impl HttpConfig {
    /// Creates a new `HttpConfig` with default settings.
    pub fn new() -> HttpConfig {
        Self { cfg: HttpAutoConfig::new(), }
    }

    /// Clones the current configuration state and returns it.
    /// Useful for building configurations dynamically.
    pub fn build(&mut self) -> Self {
        self.clone()
    }

    /// Configures whether to exclusively support HTTP/1.
    ///
    /// When enabled, only HTTP/1 requests are processed, and HTTP/2 requests are rejected.
    ///
    /// Default is `false`.
    pub fn http1_only(&mut self, val: bool) -> &mut Self {
        if val { self.cfg.http1_only(); }
        self
    }

    /// Specifies if HTTP/1 connections should be allowed to use half-closures.
    ///
    /// A half-closure in TCP occurs when one side of the data stream is terminated,
    /// but the other side remains open. This setting, when `true`, ensures the server
    /// doesn't immediately close a connection if a client shuts down their sending side
    /// while waiting for a response.
    ///
    /// Default is `false`.
    pub fn http1_half_close(&mut self, val: bool) -> &mut Self {
        self.cfg.http1.half_close(val);
        self
    }

    /// Enables or disables the keep-alive feature for HTTP/1 connections.
    ///
    /// Keep-alive allows the connection to be reused for multiple requests and responses.
    ///
    /// Default is true.
    pub fn http1_keep_alive(&mut self, val: bool) -> &mut Self {
        self.cfg.http1.keep_alive(val);
        self
    }

    /// Determines if HTTP/1 connections should write headers with title-case naming.
    ///
    /// For example, turning this setting `true` would send headers as "Content-Type" instead of "content-type".
    /// Note that this has no effect on HTTP/2 connections.
    ///
    /// Default is false.
    pub fn http1_title_case_headers(&mut self, enabled: bool) -> &mut Self {
        self.cfg.http1.title_case_headers(enabled);
        self
    }

    /// Determines if HTTP/1 connections should preserve the original case of headers.
    ///
    /// By default, headers might be normalized. Enabling this will ensure headers retain their original casing.
    /// This setting doesn't influence HTTP/2.
    ///
    /// Default is false.
    pub fn http1_preserve_header_case(&mut self, enabled: bool) -> &mut Self {
        self.cfg.http1.preserve_header_case(enabled);
        self
    }

    /// Configures a timeout for how long the server will wait for client headers.
    ///
    /// If the client doesn't send all headers within this duration, the connection is terminated.
    ///
    /// Default is None, meaning no timeout.
    pub fn http1_header_read_timeout(&mut self, val: Duration) -> &mut Self {
        self.cfg.http1.header_read_timeout(val);
        self
    }

    /// Specifies whether to use vectored writes for HTTP/1 connections.
    ///
    /// Vectored writes can be efficient for multiple non-contiguous data segments.
    /// However, certain transports (like many TLS implementations) may not handle vectored writes well.
    /// When disabled, data is flattened into a single buffer before writing.
    ///
    /// Default is `auto`, where the best method is determined dynamically.
    pub fn http1_writev(&mut self, val: bool) -> &mut Self {
        self.cfg.http1.writev(val);
        self
    }

    /// Configures the server to exclusively support HTTP/2.
    ///
    /// When enabled, only HTTP/2 requests are processed, and HTTP/1 requests are rejected.
    ///
    /// Default is false.
    pub fn http2_only(&mut self, val: bool) -> &mut Self {
        self.cfg.http2_only = true;
        self
    }

    /// Sets the [`SETTINGS_INITIAL_WINDOW_SIZE`][spec] option for HTTP2
    /// stream-level flow control.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    ///
    /// [spec]: https://http2.github.io/http2-spec/#SETTINGS_INITIAL_WINDOW_SIZE
    pub fn http2_initial_stream_window_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.cfg.http2.initial_stream_window_size(sz);
        self
    }

    /// Sets the max connection-level flow control for HTTP2.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    pub fn http2_initial_connection_window_size(
        &mut self,
        sz: impl Into<Option<u32>>,
    ) -> &mut Self {
        self.cfg.http2.initial_connection_window_size(sz);
        self
    }

    /// Sets whether to use an adaptive flow control.
    ///
    /// Enabling this will override the limits set in
    /// `http2_initial_stream_window_size` and
    /// `http2_initial_connection_window_size`.
    pub fn http2_adaptive_window(&mut self, enabled: bool) -> &mut Self {
        self.cfg.http2.adaptive_window(enabled);
        self
    }

    /// Enables the [extended CONNECT protocol].
    ///
    /// [extended CONNECT protocol]: https://datatracker.ietf.org/doc/html/rfc8441#section-4
    pub fn http2_enable_connect_protocol(&mut self) -> &mut Self {
        self.cfg.http2.enable_connect_protocol();
        self
    }

    /// Sets the maximum frame size to use for HTTP2.
    ///
    /// Passing `None` will do nothing.
    ///
    /// If not set, hyper will use a default.
    pub fn http2_max_frame_size(&mut self, sz: impl Into<Option<u32>>) -> &mut Self {
        self.cfg.http2.max_frame_size(sz);
        self
    }

    /// Sets the [`SETTINGS_MAX_CONCURRENT_STREAMS`][spec] option for HTTP2
    /// connections.
    ///
    /// Default is no limit (`std::u32::MAX`). Passing `None` will do nothing.
    ///
    /// [spec]: https://http2.github.io/http2-spec/#SETTINGS_MAX_CONCURRENT_STREAMS
    pub fn http2_max_concurrent_streams(&mut self, max: impl Into<Option<u32>>) -> &mut Self {
        self.cfg.http2.max_concurrent_streams(max);
        self
    }

    /// Sets the max size of received header frames.
    ///
    /// Default is currently ~16MB, but may change.
    pub fn http2_max_header_list_size(&mut self, max: u32) -> &mut Self {
        self.cfg.http2.max_header_list_size(max);
        self
    }

    /// Configures the maximum number of pending reset streams allowed before a GOAWAY will be sent.
    ///
    /// This will default to the default value set by the [`h2` crate](https://crates.io/crates/h2).
    /// As of v0.3.17, it is 20.
    ///
    /// See <https://github.com/hyperium/hyper/issues/2877> for more information.
    pub fn http2_max_pending_accept_reset_streams(
        &mut self,
        max: impl Into<Option<usize>>,
    ) -> &mut Self {
        self.cfg.http2.max_pending_accept_reset_streams(max);
        self
    }

    /// Set the maximum write buffer size for each HTTP/2 stream.
    ///
    /// Default is currently ~400KB, but may change.
    ///
    /// # Panics
    ///
    /// The value must be no larger than `u32::MAX`.
    pub fn http2_max_send_buf_size(&mut self, max: usize) -> &mut Self {
        self.cfg.http2.max_send_buf_size(max);
        self
    }

    /// Sets an interval for HTTP2 Ping frames should be sent to keep a
    /// connection alive.
    ///
    /// Pass `None` to disable HTTP2 keep-alive.
    ///
    /// Default is currently disabled.
    pub fn http2_keep_alive_interval(
        &mut self,
        interval: impl Into<Option<Duration>>,
    ) -> &mut Self {
        self.cfg.http2.keep_alive_interval(interval);
        self
    }

    /// Sets a timeout for receiving an acknowledgement of the keep-alive ping.
    ///
    /// If the ping is not acknowledged within the timeout, the connection will
    /// be closed. Does nothing if `http2_keep_alive_interval` is disabled.
    ///
    /// Default is 20 seconds.
    pub fn http2_keep_alive_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.cfg.http2.keep_alive_timeout(timeout);
        self
    }

    /// Set the maximum buffer size for the HTTP/1 connection.
    ///
    /// Default is ~400kb.
    ///
    /// # Panics
    ///
    /// The minimum value allowed is 8192. This method panics if the passed `max` is less than the minimum.
    pub fn max_buf_size(&mut self, max: usize) -> &mut Self {
        self.cfg.http1.max_buf_size(max);
        self
    }

    /// Determines if multiple responses should be buffered and sent together to support pipelined responses.
    ///
    /// This can improve throughput in certain situations, but is experimental and might contain issues.
    ///
    /// Default is false.
    pub fn pipeline_flush(&mut self, enabled: bool) -> &mut Self {
        self.cfg.http1.pipeline_flush(enabled);
        self
    }
}
