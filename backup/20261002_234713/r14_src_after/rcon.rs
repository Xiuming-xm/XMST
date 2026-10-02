//! Minimal Minecraft RCON client (TCP).
//! Protocol: https://wiki.vg/RCON
//! Packet: [len: i32 LE][request_id: i32 LE][type: i32 LE][payload: utf8 + \0][pad: \0\0]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const TYPE_AUTH: i32 = 3;
const TYPE_AUTH_RESP: i32 = 2;
const TYPE_EXEC: i32 = 2;
const TYPE_RESP_VALUE: i32 = 0;

pub struct RconClient {
    stream: TcpStream,
}

#[derive(Debug)]
pub struct RconError(pub String);

impl std::fmt::Display for RconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for RconError {}

impl RconClient {
    /// Connect to addr (host:port) and authenticate with password.
    pub fn connect(addr: &str, password: &str) -> Result<RconClient, RconError> {
        let mut stream = TcpStream::connect(addr).map_err(|e| RconError(format!("连接失败: {e}")))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .ok();
        let mut c = RconClient { stream };
        c.authenticate(password)?;
        Ok(c)
    }

    fn authenticate(&mut self, password: &str) -> Result<(), RconError> {
        let id = 0x7F000001_i32;
        self.write_packet(id, TYPE_AUTH, password.as_bytes())?;
        let (rid, rtype, _payload) = self.read_packet()?;
        if rtype == TYPE_AUTH_RESP && rid == id {
            Ok(())
        } else {
            Err(RconError("RCON 认证失败：密码错误或服务端未开启 enable-rcon".to_string()))
        }
    }

    /// Send a console command and return the response text.
    pub fn cmd(&mut self, command: &str) -> Result<String, RconError> {
        let id = 0x7F000002_i32;
        self.write_packet(id, TYPE_EXEC, command.as_bytes())?;
        let (rid, rtype, payload) = self.read_packet()?;
        if rid != id {
            return Err(RconError("RCON 响应 ID 不匹配".to_string()));
        }
        if rtype == TYPE_RESP_VALUE {
            let mut text = String::from_utf8_lossy(&payload).trim_end_matches('\0').to_string();
            // Some servers split long responses into multiple packets with the same id.
            // Try to drain any remaining buffered fragments (best effort, 20ms window).
            let deadline = std::time::Instant::now() + Duration::from_millis(50);
            loop {
                if std::time::Instant::now() > deadline {
                    break;
                }
                let mut probe = [0u8; 4096];
                match self.stream.peek(&mut probe) {
                    Ok(n) if n > 0 => {
                        // Best-effort: read another packet and append if same id.
                        match self.read_packet() {
                            Ok((rid2, rtype2, payload2)) if rid2 == id && rtype2 == TYPE_RESP_VALUE => {
                                text.push_str(&String::from_utf8_lossy(&payload2).trim_end_matches('\0'));
                            }
                            _ => break,
                        }
                    }
                    _ => break,
                }
            }
            Ok(text)
        } else {
            Err(RconError(format!("RCON 命令未获响应 (type={rtype})")))
        }
    }

    fn write_packet(&mut self, id: i32, rtype: i32, payload: &[u8]) -> Result<(), RconError> {
        let mut body: Vec<u8> = Vec::with_capacity(4 + 4 + payload.len() + 2);
        body.extend_from_slice(&id.to_le_bytes());
        body.extend_from_slice(&rtype.to_le_bytes());
        body.extend_from_slice(payload);
        body.push(0); // null terminator
        body.extend_from_slice(&[0u8, 0u8]); // pad
        let len = body.len() as i32;
        let mut buf = Vec::with_capacity(4 + body.len());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&body);
        self.stream.write_all(&buf).map_err(|e| RconError(format!("发送失败: {e}")))
    }

    fn read_packet(&mut self) -> Result<(i32, i32, Vec<u8>), RconError> {
        let mut len_buf = [0u8; 4];
        self.stream
            .read_exact(&mut len_buf)
            .map_err(|e| RconError(format!("读取包头失败: {e}")))?;
        let len = i32::from_le_bytes(len_buf);
        if len < 8 || len > 16 * 1024 * 1024 {
            return Err(RconError(format!("非法 RCON 包长度: {len}")));
        }
        // length 字段不含自身：标准包体 = id(4) + type(4) + payload + pad(2)。
        // 此前误用 len-4，认证失败响应(length=10) 只分配 6 字节，
        // 导致「RCON 包体过短: 6 字节」与 body[8..] 越界 panic。
        let mut body = vec![0u8; len as usize];
        self.stream
            .read_exact(&mut body)
            .map_err(|e| RconError(format!("读取包体失败: {e}")))?;
        // 防越界：包体可能不足 8 字节（异常/伪造响应），此前 body[8..] 在此 panic，
        // 子线程 panic 会触发全局 panic hook 弹窗，造成「崩溃提示但工具未崩溃」的误报
        if body.len() < 8 {
            return Err(RconError(format!("RCON 包体过短: {} 字节", body.len())));
        }
        let id = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let rtype = i32::from_le_bytes([body[4], body[5], body[6], body[7]]);
        let payload = body[8..].to_vec();
        Ok((id, rtype, payload))
    }
}

/// One-shot helper: connect, auth, run commands in order, return last response.
pub fn query(host: &str, port: u16, password: &str, commands: &[&str]) -> Result<Vec<String>, RconError> {
    let addr = format!("{host}:{port}");
    let mut c = RconClient::connect(&addr, password)?;
    let mut out = Vec::new();
    for cmd in commands {
        out.push(c.cmd(cmd)?);
    }
    Ok(out)
}
