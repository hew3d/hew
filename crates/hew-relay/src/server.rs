//! One accepted connection: what it may hold, and for how long. The relay
//! cannot tell clients apart (behind the proxy they all arrive from one
//! address), so what a connection holds outside the store's byte budget is
//! bounded here, and so is how long a stalled one can hold anything:
//!
//! - HTTP/1 only, one request at a time per connection.
//! - A request head must arrive within `header_read_timeout`, counted from
//!   the accept or from the previous response.
//! - The request body and handler are bounded by `http.rs`'s timeout.
//! - A response must keep moving: see [`WriteDeadline`].
//! - The connection's read buffer stops growing at [`MAX_BUF_SIZE`].

use std::future::Future;
use std::io::{self, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Instant, Sleep};

/// Cap on a connection's read buffer, which holds the request head and body
/// bytes not yet consumed. It lives outside the byte budget, so it is far
/// below hyper's ~400 KiB default; request heads here are tiny.
pub const MAX_BUF_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// How long a connection may sit between the TCP accept (or the previous
    /// response, on keep-alive) and a complete request head. This is the
    /// slow-loris guard; the request-body/handler budget is `http.rs`'s
    /// `REQUEST_TIMEOUT`, which only starts once the head has been parsed.
    pub header_read_timeout: Duration,
    /// While a response is waiting on the client, each window of this length
    /// must see the socket accept `write_min_bytes` more...
    pub write_window: Duration,
    /// ...or the connection is closed, releasing the response's quota.
    pub write_min_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        // 1 MiB per 2 minutes. The window is long because the kernel does not
        // report progress smoothly: with a send buffer grown to several MiB,
        // a blocked writer is woken only after about a third of it drains, so
        // a short window would cut off a slow but steady download. With
        // Linux's default send buffer limit, a reader averaging 16 KiB/s
        // or more stays inside these numbers; a much larger limit can cut
        // one off. A 32 MiB response is held open for 64 minutes at most.
        Self {
            header_read_timeout: Duration::from_secs(15),
            write_window: Duration::from_secs(120),
            write_min_bytes: 1024 * 1024,
        }
    }
}

/// Fails a connection whose peer stops taking the response, or takes it
/// more slowly than the floor in [`Limits`]. It is a floor on progress, not
/// a deadline for the whole response: a download that keeps up may take as
/// long as it needs.
///
/// The clock runs only from the first write the socket cannot accept until
/// the next completed flush (hyper flushes the socket once its own buffer is
/// empty), so an idle connection is never subject to it.
pub struct WriteDeadline<T> {
    inner: T,
    window: Duration,
    min_bytes: usize,
    timer: Pin<Box<Sleep>>,
    armed: bool,
    /// Bytes accepted in the current window.
    moved: usize,
}

impl<T> WriteDeadline<T> {
    pub fn new(inner: T, window: Duration, min_bytes: usize) -> Self {
        Self {
            inner,
            window,
            min_bytes,
            timer: Box::pin(tokio::time::sleep(window)),
            armed: false,
            moved: 0,
        }
    }

    fn restart_window(&mut self) {
        self.moved = 0;
        self.timer.as_mut().reset(Instant::now() + self.window);
    }

    fn track(
        &mut self,
        cx: &mut Context<'_>,
        result: Poll<io::Result<usize>>,
    ) -> Poll<io::Result<usize>> {
        match result {
            Poll::Ready(Ok(n)) => {
                if self.armed {
                    self.moved = self.moved.saturating_add(n);
                    if self.moved >= self.min_bytes {
                        self.restart_window();
                    }
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => {
                if !self.armed {
                    self.armed = true;
                    self.restart_window();
                }
                // Polling the timer is also what wakes this task when a
                // fully stalled peer never makes the socket writable again.
                match self.timer.as_mut().poll(cx) {
                    Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "response write too slow",
                    ))),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for WriteDeadline<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for WriteDeadline<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        this.track(cx, result)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write_vectored(cx, bufs);
        this.track(cx, result)
    }

    // Must be forwarded: without vectored writes hyper copies the whole
    // response body into its own buffer, outside the byte budget.
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        if result.is_ready() {
            self.armed = false;
        }
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Serves one connection under `limits`, on its own task.
///
/// The accept loop is hand-rolled over this rather than `axum::serve`: the
/// latter installs no timer on the connection builder, which silently
/// disables hyper's `header_read_timeout`. HTTP/1 is named outright because
/// protocol auto-detection waits for the first bytes with no timer at all,
/// and an HTTP/2 connection has neither this timeout nor one request at a
/// time. The relay's clients and its proxy all speak HTTP/1.1.
pub fn spawn_connection<I>(io: I, router: Router, limits: Limits, graceful: &GracefulShutdown)
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let io = WriteDeadline::new(io, limits.write_window, limits.write_min_bytes);
    let conn = graceful.watch(
        http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(limits.header_read_timeout)
            .max_buf_size(MAX_BUF_SIZE)
            .serve_connection(TokioIo::new(io), TowerToHyperService::new(router)),
    );
    tokio::spawn(async move {
        if let Err(err) = conn.await {
            // Client went away mid-request, malformed HTTP, a timeout —
            // none of it is actionable here.
            tracing::debug!(error = %err, "connection ended with error");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{self, AppState};
    use crate::store::Store;
    use std::collections::HashSet;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::time::{sleep, timeout};

    const WINDOW: Duration = Duration::from_millis(400);
    const FLOOR: usize = 4096;

    /// A `WriteDeadline` writing `len` bytes into a 1 KiB pipe whose reader
    /// takes `chunk` bytes every `pause`. Returns what the writer saw.
    async fn write_to_reader(len: usize, chunk: usize, pause: Duration) -> io::Result<()> {
        let (near, mut far) = tokio::io::duplex(1024);
        let reader = tokio::spawn(async move {
            let mut buf = vec![0u8; chunk.max(1)];
            loop {
                sleep(pause).await;
                if chunk > 0 && far.read(&mut buf).await.unwrap_or(0) == 0 {
                    break;
                }
            }
        });
        let mut writer = WriteDeadline::new(near, WINDOW, FLOOR);
        let result = timeout(Duration::from_secs(20), async {
            writer.write_all(&vec![1; len]).await?;
            writer.flush().await
        })
        .await
        .expect("the write neither finished nor was cut off");
        reader.abort();
        result
    }

    #[tokio::test]
    async fn a_stalled_write_fails() {
        let err = write_to_reader(64 * 1024, 0, Duration::from_millis(10))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn a_trickle_below_the_floor_fails() {
        // Progress well inside every window, but about 1 KiB per window.
        let err = write_to_reader(64 * 1024, 128, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn a_slow_download_above_the_floor_outlasts_many_windows() {
        // 1 KiB every 25 ms: 16 KiB per window against a floor of 4 KiB,
        // and 96 KiB takes about six windows.
        let started = Instant::now();
        write_to_reader(96 * 1024, 1024, Duration::from_millis(25))
            .await
            .unwrap();
        assert!(started.elapsed() > WINDOW * 3);
    }

    #[tokio::test]
    async fn an_idle_flushed_connection_has_no_write_deadline() {
        let (near, mut far) = tokio::io::duplex(8);
        let mut writer = WriteDeadline::new(near, WINDOW, FLOOR);
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 8];
            for _ in 0..2 {
                sleep(Duration::from_millis(20)).await;
                far.read_exact(&mut buf).await.unwrap();
            }
            far
        });
        // Blocks once (arming the clock), then flushes.
        writer.write_all(&[0; 16]).await.unwrap();
        writer.flush().await.unwrap();
        let _far = reader.await.unwrap();
        sleep(WINDOW * 2).await;
        // The pipe is empty again; fill it, then block anew with a fresh
        // window rather than one that expired while idle.
        writer.write_all(&[0; 8]).await.unwrap();
        assert!(
            timeout(WINDOW / 4, writer.write_all(&[0; 1]))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn vectored_writes_survive_the_wrapper() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        assert!(stream.is_write_vectored());
        assert!(WriteDeadline::new(stream, WINDOW, FLOOR).is_write_vectored());
    }

    struct Server {
        addr: std::net::SocketAddr,
        state: Arc<AppState>,
        accept: tokio::task::JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.accept.abort();
        }
    }

    async fn serve(state: AppState, limits: Limits) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(state);
        let router = http::router(Arc::clone(&state));
        let accept = tokio::spawn(async move {
            let graceful = GracefulShutdown::new();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                spawn_connection(stream, router.clone(), limits, &graceful);
            }
        });
        Server {
            addr,
            state,
            accept,
        }
    }

    fn app_state(max_bytes: usize, total: usize) -> AppState {
        AppState {
            store: Arc::new(Store::new(total, Duration::from_millis(200))),
            max_bytes,
            allowed_origins: HashSet::new(),
            upload_key: None,
        }
    }

    fn short_limits() -> Limits {
        Limits {
            header_read_timeout: Duration::from_millis(300),
            write_window: WINDOW,
            write_min_bytes: FLOOR,
        }
    }

    #[tokio::test]
    async fn a_stalled_get_reader_returns_its_quota() {
        // Larger than loopback's socket buffers can swallow on either side.
        const LEN: usize = 32 * 1024 * 1024;
        let server = serve(app_state(LEN, LEN), short_limits()).await;
        assert!(server.state.store.reserve(LEN));
        let token = server.state.store.put_reserved(vec![5; LEN], LEN);

        let mut client = TcpStream::connect(server.addr).await.unwrap();
        let request = format!("GET /drop/{token} HTTP/1.1\r\nhost: relay\r\n\r\n");
        client.write_all(request.as_bytes()).await.unwrap();
        // Read a little, so the response is known to be under way, then stop.
        client.read_exact(&mut [0u8; 1024]).await.unwrap();
        assert_eq!(server.state.store.remaining(), 0);

        timeout(Duration::from_secs(10), async {
            while server.state.store.remaining() != LEN {
                sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("a stalled reader still holds the quota");
        drop(client);
    }

    /// Reads until the server closes the connection, or panics.
    async fn read_until_closed(client: &mut TcpStream) -> Vec<u8> {
        let mut seen = Vec::new();
        // A reset also counts as closed.
        let _ = timeout(Duration::from_secs(5), client.read_to_end(&mut seen))
            .await
            .expect("the server kept the connection open");
        seen
    }

    #[tokio::test]
    async fn a_silent_or_half_spoken_connection_is_closed() {
        let server = serve(app_state(16, 4096), short_limits()).await;
        // Nothing at all, and the start of what could be an HTTP/2 preface.
        for opening in ["", "PRI * HT"] {
            let mut client = TcpStream::connect(server.addr).await.unwrap();
            client.write_all(opening.as_bytes()).await.unwrap();
            read_until_closed(&mut client).await;
        }
    }

    #[tokio::test]
    async fn http2_is_not_served() {
        let server = serve(app_state(16, 4096), short_limits()).await;
        let mut client = TcpStream::connect(server.addr).await.unwrap();
        // The preface and an empty SETTINGS frame.
        client
            .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00")
            .await
            .unwrap();
        let seen = read_until_closed(&mut client).await;
        // An HTTP/2 server would open with its own SETTINGS frame (type 4).
        assert_ne!(seen.get(3), Some(&4), "{seen:?}");
    }

    #[tokio::test]
    async fn an_oversized_request_head_is_refused_at_the_buffer_cap() {
        let server = serve(app_state(16, 4096), short_limits()).await;
        let mut client = TcpStream::connect(server.addr).await.unwrap();
        let request = format!(
            "GET / HTTP/1.1\r\nhost: relay\r\nconnection: close\r\nx-pad: {}\r\n\r\n",
            "a".repeat(2 * MAX_BUF_SIZE)
        );
        // The server may refuse before the whole head is written.
        let _ = client.write_all(request.as_bytes()).await;
        let seen = read_until_closed(&mut client).await;
        assert!(!seen.starts_with(b"HTTP/1.1 200"), "head was accepted");
    }
}
