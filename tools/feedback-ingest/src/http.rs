use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
    Other,
}

#[derive(Debug)]
pub struct Request {
    pub method: Method,
    pub target: String,
    pub peer: SocketAddr,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug)]
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn text(status: u16, body: &str) -> Self {
        Self::bytes(
            status,
            body.as_bytes().to_vec(),
            "text/plain; charset=utf-8",
        )
    }

    pub fn bytes(status: u16, body: Vec<u8>, content_type: &str) -> Self {
        Self::empty(status)
            .header("Content-Type", content_type)
            .with_body(body)
    }

    fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub header_bytes: usize,
    pub header_count: usize,
    pub body_bytes: usize,
    pub read_deadline: Duration,
    pub write_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            header_bytes: 8 * 1024,
            header_count: 64,
            body_bytes: 64 * 1024,
            read_deadline: Duration::from_secs(15),
            write_timeout: Duration::from_secs(15),
        }
    }
}

pub type Handler = Arc<dyn Fn(Request) -> Response + Send + Sync>;

pub fn serve(
    listener: TcpListener,
    workers: usize,
    limits: Limits,
    handler: Handler,
) -> io::Result<()> {
    let (tx, rx) = mpsc::sync_channel::<TcpStream>(workers.max(1) * 4);
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..workers.max(1) {
        let rx = Arc::clone(&rx);
        let handler = Arc::clone(&handler);
        thread::spawn(move || loop {
            let next = rx.lock().map(|r| r.recv()).unwrap_or(Err(mpsc::RecvError));
            match next {
                Ok(stream) => handle_connection(stream, limits, &handler),
                Err(_) => return,
            }
        });
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        if let Err(TrySendError::Full(stream)) | Err(TrySendError::Disconnected(stream)) =
            tx.try_send(stream)
        {
            let _ = stream.set_write_timeout(Some(limits.write_timeout));
            let mut stream = stream;
            let _ = write_response(&mut stream, &Response::text(503, "busy"), false);
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, limits: Limits, handler: &Handler) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(limits.write_timeout));
    let outcome = read_request(&mut stream, limits);
    let _ = match outcome {
        Ok(req) => {
            let head_only = req.method == Method::Head;
            let resp = handler(req);
            write_response(&mut stream, &resp, head_only)
        }
        Err(ReadError::Reject(status)) => {
            write_response(&mut stream, &Response::empty(status), false)
        }
        Err(ReadError::Io) => Ok(()),
    };
    let _ = stream.shutdown(Shutdown::Both);
}

enum ReadError {
    Reject(u16),
    Io,
}

fn read_request(stream: &mut TcpStream, limits: Limits) -> Result<Request, ReadError> {
    let peer = stream.peer_addr().map_err(|_| ReadError::Io)?;
    let started = Instant::now();
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() >= limits.header_bytes {
            return Err(ReadError::Reject(431));
        }
        let n = timed_read(stream, &mut chunk, started, limits.read_deadline)?;
        buf.extend_from_slice(&chunk[..n]);
    };

    let head = std::str::from_utf8(&buf[..head_end]).map_err(|_| ReadError::Reject(400))?;
    let (method, target, headers) =
        parse_head(head, limits.header_count).map_err(ReadError::Reject)?;

    if headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("Transfer-Encoding"))
    {
        return Err(ReadError::Reject(411));
    }
    let content_length = match headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
    {
        None => 0,
        Some((_, v)) => v
            .trim()
            .parse::<usize>()
            .map_err(|_| ReadError::Reject(400))?,
    };
    if content_length > limits.body_bytes {
        return Err(ReadError::Reject(413));
    }

    let mut body = buf.split_off(head_end + 4);
    if body.len() > content_length {
        return Err(ReadError::Reject(400));
    }
    while body.len() < content_length {
        let want = (content_length - body.len()).min(chunk.len());
        let n = timed_read(stream, &mut chunk[..want], started, limits.read_deadline)?;
        body.extend_from_slice(&chunk[..n]);
    }

    Ok(Request {
        method,
        target,
        peer,
        headers,
        body,
    })
}

fn timed_read(
    stream: &mut TcpStream,
    buf: &mut [u8],
    started: Instant,
    deadline: Duration,
) -> Result<usize, ReadError> {
    let left = deadline.saturating_sub(started.elapsed());
    if left.is_zero() {
        return Err(ReadError::Io);
    }
    stream
        .set_read_timeout(Some(left))
        .map_err(|_| ReadError::Io)?;
    match stream.read(buf) {
        Ok(0) => Err(ReadError::Io),
        Ok(n) => Ok(n),
        Err(_) => Err(ReadError::Io),
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

type Head = (Method, String, Vec<(String, String)>);

fn parse_head(head: &str, max_headers: usize) -> Result<Head, u16> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or(400u16)?;
    let mut parts = request_line.split(' ');
    let method = match parts.next().ok_or(400u16)? {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        "POST" => Method::Post,
        "" => return Err(400),
        _ => Method::Other,
    };
    let target = parts.next().filter(|t| !t.is_empty()).ok_or(400u16)?;
    let version = parts.next().ok_or(400u16)?;
    if !version.starts_with("HTTP/1.") || parts.next().is_some() {
        return Err(400);
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if headers.len() >= max_headers {
            return Err(431);
        }
        let (name, value) = line.split_once(':').ok_or(400u16)?;
        if name.is_empty() || name.contains(|c: char| c.is_whitespace()) {
            return Err(400);
        }
        headers.push((name.to_string(), value.trim().to_string()));
    }
    Ok((method, target.to_string(), headers))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn write_response(stream: &mut TcpStream, resp: &Response, head_only: bool) -> io::Result<()> {
    let mut out = Vec::with_capacity(256 + resp.body.len());
    out.extend_from_slice(
        format!(
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
            resp.status,
            reason(resp.status),
            resp.body.len()
        )
        .as_bytes(),
    );
    for (k, v) in &resp.headers {
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"\r\n");
    if !head_only {
        out.extend_from_slice(&resp.body);
    }
    stream.write_all(&out)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn start(limits: Limits, handler: Handler) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || serve(listener, 2, limits, handler));
        addr
    }

    fn talk(addr: SocketAddr, raw: &[u8]) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(raw).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    fn counting_handler() -> (Handler, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let handler: Handler = Arc::new(move |req: Request| {
            c.fetch_add(1, Ordering::SeqCst);
            Response::text(
                200,
                &format!("{} {} {}", req.method_name(), req.target, req.body.len()),
            )
        });
        (handler, calls)
    }

    impl Request {
        fn method_name(&self) -> &'static str {
            match self.method {
                Method::Get => "GET",
                Method::Head => "HEAD",
                Method::Post => "POST",
                Method::Other => "OTHER",
            }
        }
    }

    #[test]
    fn oversize_content_length_is_rejected_without_reading_body() {
        let (handler, calls) = counting_handler();
        let addr = start(Limits::default(), handler);
        let started = Instant::now();
        let reply = talk(
            addr,
            b"POST /x HTTP/1.1\r\nHost: a\r\nContent-Length: 1000000000000\r\n\r\n",
        );
        assert!(reply.starts_with("HTTP/1.1 413 "), "{reply}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn post_body_round_trips() {
        let (handler, calls) = counting_handler();
        let addr = start(Limits::default(), handler);
        let reply = talk(
            addr,
            b"POST /ping?x=1 HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\nhello",
        );
        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
        assert!(reply.contains("Connection: close\r\n"));
        assert!(reply.ends_with("POST /ping?x=1 5"), "{reply}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn head_omits_body_but_keeps_length() {
        let (handler, _) = counting_handler();
        let addr = start(Limits::default(), handler);
        let reply = talk(addr, b"HEAD / HTTP/1.1\r\nHost: a\r\n\r\n");
        assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
        assert!(reply.contains("Content-Length: 8\r\n"), "{reply}");
        assert!(reply.ends_with("\r\n\r\n"), "{reply}");
    }

    #[test]
    fn huge_headers_are_rejected() {
        let (handler, calls) = counting_handler();
        let limits = Limits {
            header_bytes: 512,
            ..Limits::default()
        };
        let addr = start(limits, handler);
        let mut raw = b"GET / HTTP/1.1\r\n".to_vec();
        raw.extend_from_slice(format!("X-Pad: {}\r\n\r\n", "a".repeat(1024)).as_bytes());
        let reply = talk(addr, &raw);
        assert!(reply.starts_with("HTTP/1.1 431 "), "{reply}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn chunked_bodies_are_refused() {
        let (handler, calls) = counting_handler();
        let addr = start(Limits::default(), handler);
        let reply = talk(
            addr,
            b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        );
        assert!(reply.starts_with("HTTP/1.1 411 "), "{reply}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn slow_client_is_dropped_at_deadline() {
        let (handler, calls) = counting_handler();
        let limits = Limits {
            read_deadline: Duration::from_millis(300),
            ..Limits::default()
        };
        let addr = start(limits, handler);
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc")
            .unwrap();
        let started = Instant::now();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn parse_head_accepts_well_formed_requests() {
        let (m, t, h) = parse_head(
            "POST /deviceout-feedback/ping HTTP/1.1\r\nHost: x\r\nX-DeviceOut-Token:  abc \r\n",
            64,
        )
        .unwrap();
        assert_eq!(m, Method::Post);
        assert_eq!(t, "/deviceout-feedback/ping");
        assert_eq!(
            h,
            vec![
                ("Host".into(), "x".into()),
                ("X-DeviceOut-Token".into(), "abc".into())
            ]
        );
    }

    #[test]
    fn parse_head_rejects_malformed_requests() {
        assert_eq!(parse_head("GET\r\n", 64).unwrap_err(), 400);
        assert_eq!(parse_head("GET / HTTP/2\r\n", 64).unwrap_err(), 400);
        assert_eq!(parse_head("GET / HTTP/1.1 extra\r\n", 64).unwrap_err(), 400);
        assert_eq!(
            parse_head("GET / HTTP/1.1\r\nno-colon\r\n", 64).unwrap_err(),
            400
        );
        assert_eq!(
            parse_head("GET / HTTP/1.1\r\nBad Name: v\r\n", 64).unwrap_err(),
            400
        );
        assert_eq!(
            parse_head("GET / HTTP/1.1\r\nA: 1\r\nB: 2\r\n", 1).unwrap_err(),
            431
        );
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let req = Request {
            method: Method::Get,
            target: "/".into(),
            peer: "127.0.0.1:1".parse().unwrap(),
            headers: vec![("Content-Length".into(), "3".into())],
            body: Vec::new(),
        };
        assert_eq!(req.header("content-length"), Some("3"));
        assert_eq!(req.header("x-none"), None);
    }
}
