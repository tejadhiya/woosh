//! Minimal synchronous HTTP/1.1 client over Unix sockets and TCP.
//!
//! Exists so the Docker and Ollama domains can use real APIs instead of parsing
//! CLI stdout, **without** dragging `tokio`/`hyper` into what is otherwise a
//! synchronous rayon engine. The prototype's string parsers are the thing being
//! replaced here:
//!
//! * `_parse_docker_size("1.5GB (50%)")` → float arithmetic on a display string.
//! * Ollama sizes rebuilt from `parts[2] + parts[3]` of `ollama list` output.
//!
//! Both are now exact integers out of JSON. ~150 lines beats two fragile parsers
//! and an async runtime.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(8);

/// Where to connect.
pub enum Endpoint<'a> {
    /// Unix domain socket, e.g. `/var/run/docker.sock`.
    Unix(&'a Path),
    /// `host:port`, e.g. `127.0.0.1:11434`.
    Tcp(&'a str),
}

#[derive(Debug)]
pub enum HttpError {
    Connect(String),
    Io(String),
    Status(u16, String),
    Malformed(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Connect(e) => write!(f, "connect failed: {e}"),
            HttpError::Io(e) => write!(f, "io error: {e}"),
            HttpError::Status(c, b) => write!(f, "http {c}: {b}"),
            HttpError::Malformed(e) => write!(f, "malformed response: {e}"),
        }
    }
}

/// Either socket flavour behind one interface.
enum Conn {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Unix(s) => s.read(buf),
            Conn::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Unix(s) => s.write(buf),
            Conn::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Unix(s) => s.flush(),
            Conn::Tcp(s) => s.flush(),
        }
    }
}

fn connect(ep: &Endpoint<'_>) -> Result<Conn, HttpError> {
    match ep {
        Endpoint::Unix(p) => {
            let s = UnixStream::connect(p).map_err(|e| HttpError::Connect(e.to_string()))?;
            s.set_read_timeout(Some(TIMEOUT)).ok();
            s.set_write_timeout(Some(TIMEOUT)).ok();
            Ok(Conn::Unix(s))
        }
        Endpoint::Tcp(addr) => {
            let s = TcpStream::connect(addr).map_err(|e| HttpError::Connect(e.to_string()))?;
            s.set_read_timeout(Some(TIMEOUT)).ok();
            s.set_write_timeout(Some(TIMEOUT)).ok();
            Ok(Conn::Tcp(s))
        }
    }
}

/// Issue a request and return the response body.
///
/// `Connection: close` keeps framing simple: for responses without
/// `Content-Length` or `Transfer-Encoding: chunked`, EOF terminates the body.
pub fn request(
    ep: Endpoint<'_>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<String, HttpError> {
    let host = match &ep {
        Endpoint::Unix(_) => "localhost",
        Endpoint::Tcp(a) => a,
    };
    let mut conn = connect(&ep)?;

    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n"
    );
    if let Some(b) = body {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }

    conn.write_all(req.as_bytes())
        .map_err(|e| HttpError::Io(e.to_string()))?;
    conn.flush().map_err(|e| HttpError::Io(e.to_string()))?;

    let mut reader = BufReader::new(conn);

    // Status line
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| HttpError::Io(e.to_string()))?;
    let code: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| HttpError::Malformed(format!("bad status line: {line:?}")))?;

    // Headers
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let mut h = String::new();
        let n = reader
            .read_line(&mut h)
            .map_err(|e| HttpError::Io(e.to_string()))?;
        if n == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        let lower = h.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().ok();
        } else if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }

    // Body
    let body = if chunked {
        read_chunked(&mut reader)?
    } else if let Some(len) = content_length {
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| HttpError::Io(e.to_string()))?;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        let mut s = String::new();
        reader
            .read_to_string(&mut s)
            .map_err(|e| HttpError::Io(e.to_string()))?;
        s
    };

    if !(200..300).contains(&code) {
        return Err(HttpError::Status(code, body.chars().take(400).collect()));
    }
    Ok(body)
}

fn read_chunked<R: BufRead>(reader: &mut R) -> Result<String, HttpError> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        reader
            .read_line(&mut size_line)
            .map_err(|e| HttpError::Io(e.to_string()))?;
        // Chunk extensions after ';' are legal and ignored.
        let hex = size_line.trim().split(';').next().unwrap_or("").trim();
        if hex.is_empty() {
            break;
        }
        let size = usize::from_str_radix(hex, 16)
            .map_err(|_| HttpError::Malformed(format!("bad chunk size {hex:?}")))?;
        if size == 0 {
            break;
        }
        let mut chunk = vec![0u8; size];
        reader
            .read_exact(&mut chunk)
            .map_err(|e| HttpError::Io(e.to_string()))?;
        out.extend_from_slice(&chunk);
        // Trailing CRLF after each chunk.
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}
