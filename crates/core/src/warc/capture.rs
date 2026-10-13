use std::io::Write;

use anyhow::Result;
use flate2::{write::GzEncoder, Compression};

use crate::capture::{sha256, Capture};

struct Count<'a, W> {
    output: &'a mut W,
    bytes: u64,
}
impl<W: Write> Write for Count<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let size = self.output.write(bytes)?;
        self.bytes += size as u64;
        Ok(size)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

fn base32(bytes: &[u8]) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::new();
    let mut bits = 0u32;
    let mut count = 0;
    for byte in bytes {
        bits = (bits << 8) | *byte as u32;
        count += 8;
        while count >= 5 {
            count -= 5;
            output.push(alphabet[((bits >> count) & 31) as usize] as char);
        }
    }
    if count > 0 {
        output.push(alphabet[((bits << (5 - count)) & 31) as usize] as char);
    }
    while output.len() % 8 != 0 {
        output.push('=');
    }
    output
}

fn digest(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        base32(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
    )
}

fn header<W: Write>(
    writer: &mut W,
    kind: &str,
    id: &str,
    date: chrono::DateTime<chrono::Utc>,
    size: usize,
) -> Result<()> {
    write!(writer, "WARC/1.1\r\nWARC-Type: {kind}\r\nWARC-Record-ID: <urn:uuid:{id}>\r\nWARC-Date: {}\r\nContent-Length: {size}\r\n", date.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))?;
    Ok(())
}

pub fn write<W: Write>(
    output: &mut W,
    capture: &Capture,
    payload_type: super::PayloadType,
    level: u32,
    include_info: bool,
) -> Result<u64> {
    let meta = &capture.metadata;
    meta.validate()?;
    anyhow::ensure!(
        capture.payload.len() == meta.payload_bytes
            && sha256(&capture.payload) == meta.payload_sha256,
        "capture payload mismatch before append"
    );
    let mut output = Count { output, bytes: 0 };
    if include_info {
        let content = b"software: AstianGO\r\nformat: WARC File Format 1.1\r\nastiango-capture-version: 1\r\n";
        let mut member = GzEncoder::new(&mut output, Compression::new(level));
        header(
            &mut member,
            "warcinfo",
            &uuid::Uuid::new_v4().to_string(),
            meta.response.received_at,
            content.len(),
        )?;
        write!(member, "Content-Type: application/warc-fields\r\n\r\n")?;
        member.write_all(content)?;
        member.write_all(b"\r\n\r\n")?;
        member.finish()?;
    }
    let response_offset = output.bytes;
    let version = match meta.response.version.as_str() {
        "HTTP/1.0" => "HTTP/1.0",
        _ => "HTTP/1.1",
    };
    let mut http = format!("{version} {} \r\n", meta.response.status).into_bytes();
    for item in &meta.response.headers {
        if item.name.eq_ignore_ascii_case("transfer-encoding")
            || item.name.eq_ignore_ascii_case("content-length")
            || item.name.eq_ignore_ascii_case("trailer")
        {
            continue;
        }
        let value = item.bytes()?;
        anyhow::ensure!(
            !item.name.contains(['\r', '\n', ':'])
                && !value.contains(&b'\r')
                && !value.contains(&b'\n'),
            "invalid captured HTTP header"
        );
        http.extend_from_slice(item.name.as_bytes());
        http.extend_from_slice(b": ");
        http.extend_from_slice(&value);
        http.extend_from_slice(b"\r\n");
    }
    if meta.response.status != 304 {
        http.extend_from_slice(format!("content-length: {}\r\n", capture.payload.len()).as_bytes());
    }
    http.extend_from_slice(b"\r\n");
    let mut block_digest = ring::digest::Context::new(&ring::digest::SHA256);
    block_digest.update(&http);
    block_digest.update(&capture.payload);
    let mut member = GzEncoder::new(&mut output, Compression::new(level));
    header(
        &mut member,
        "response",
        &meta.fetch_id,
        meta.response.received_at,
        http.len() + capture.payload.len(),
    )?;
    write!(member, "WARC-Target-URI: {}\r\nContent-Type: application/http; msgtype=response\r\nWARC-Identified-Payload-Type: {payload_type}\r\nWARC-Payload-Digest: {}\r\nWARC-Block-Digest: sha256:{}\r\nAstian-Capture-Version: 1\r\n", meta.response.url, digest(&capture.payload), base32(block_digest.finish().as_ref()))?;
    if let Some(ip) = meta.response.peer_ip {
        write!(member, "WARC-IP-Address: {ip}\r\n")?;
    }
    member.write_all(b"\r\n")?;
    member.write_all(&http)?;
    member.write_all(&capture.payload)?;
    member.write_all(b"\r\n\r\n")?;
    member.finish()?;
    let bytes = serde_json::to_vec(meta)?;
    anyhow::ensure!(bytes.len() <= 1024 * 1024, "capture metadata exceeds limit");
    let mut member = GzEncoder::new(&mut output, Compression::new(level));
    header(
        &mut member,
        "metadata",
        &uuid::Uuid::new_v4().to_string(),
        meta.response.received_at,
        bytes.len(),
    )?;
    write!(member, "WARC-Target-URI: {}\r\nWARC-Concurrent-To: <urn:uuid:{}>\r\nContent-Type: application/json\r\nAstian-Capture-Version: 1\r\nWARC-Block-Digest: {}\r\n\r\n", meta.response.url, meta.fetch_id, digest(&bytes))?;
    member.write_all(&bytes)?;
    member.write_all(b"\r\n\r\n")?;
    member.finish()?;
    Ok(response_offset)
}
