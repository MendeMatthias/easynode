//! A version handshake with a BTX peer, and nothing past it.
//!
//! The probe connects, sends `version`, reads until it has the peer's
//! `version` and `verack`, answers `verack`, and closes. It never sends
//! `getdata`, `getaddr`, `getheaders` or anything else that asks the peer
//! for data: the question is only "does this seed answer a fresh node".
//!
//! The wire format is the engine's, read from btx v0.34.12:
//!   * mainnet magic `b7 54 58 01`, port 19335 (src/kernel/chainparams.cpp)
//!   * PROTOCOL_VERSION 800002, MIN_PEER_PROTO_VERSION 800001
//!     (src/node/protocol_version.h)
//!   * the header: magic, 12-byte command, LE u32 length, first four bytes of
//!     double SHA-256 of the payload (src/protocol.h, src/net.cpp)
//!   * `version`: version i32, services u64, time i64, addr_you (services,
//!     16-byte address, BE port), addr_me (same), nonce u64, user agent as a
//!     compact-size string, start height i32, relay bool
//!     (net_processing.cpp `PushNodeVersion`).
//!
//! Peers that advertise P2P_V2 still accept a v1 handshake: the engine falls
//! back when the first bytes are not a v2 key, and this sends v1 only.

use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const MAINNET_MAGIC: [u8; 4] = [0xb7, 0x54, 0x58, 0x01];
pub const PROTOCOL_VERSION: i32 = 800_002;
/// The largest payload the probe reads. A `version` is about a hundred bytes;
/// anything else before `verack` (`sendaddrv2`, `wtxidrelay`, `sendcmpct`…)
/// is tiny too. A peer announcing more is not answering a handshake.
const MAX_PAYLOAD: u32 = 64 * 1024;
/// Messages read before giving up on seeing both `version` and `verack`.
const MAX_MESSAGES: usize = 32;

/// What a peer said about itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Handshake {
    pub version: i32,
    pub services: u64,
    pub user_agent: String,
    pub start_height: i32,
}

fn checksum(payload: &[u8]) -> [u8; 4] {
    let h = Sha256::digest(Sha256::digest(payload));
    [h[0], h[1], h[2], h[3]]
}

/// One framed message.
pub fn frame(magic: [u8; 4], command: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + payload.len());
    out.extend_from_slice(&magic);
    let mut cmd = [0u8; 12];
    cmd[..command.len()].copy_from_slice(command.as_bytes());
    out.extend_from_slice(&cmd);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&checksum(payload));
    out.extend_from_slice(payload);
    out
}

/// Our `version` payload: no services, start height 0, no relay, an honest
/// user agent so a peer operator reading its log sees what dialled.
pub fn version_payload(now: i64, nonce: u64, user_agent: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(110);
    p.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    p.extend_from_slice(&0u64.to_le_bytes()); // our services
    p.extend_from_slice(&now.to_le_bytes());
    for _ in 0..2 {
        // addr_you then addr_me: services, an all-zero IPv6 address, port 0.
        // The engine ignores addr_me and only logs addr_you.
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&[0u8; 16]);
        p.extend_from_slice(&0u16.to_be_bytes());
    }
    p.extend_from_slice(&nonce.to_le_bytes());
    p.push(user_agent.len() as u8);
    p.extend_from_slice(user_agent.as_bytes());
    p.extend_from_slice(&0i32.to_le_bytes()); // start height
    p.push(0); // relay: no transactions, please
    p
}

/// Read the fields the probe reports out of a peer's `version` payload.
pub fn parse_version(p: &[u8]) -> Option<Handshake> {
    let mut r = Reader { b: p, at: 0 };
    let version = i32::from_le_bytes(r.take(4)?.try_into().ok()?);
    let services = u64::from_le_bytes(r.take(8)?.try_into().ok()?);
    r.take(8)?; // time
    r.take(26)?; // addr_you
    r.take(26)?; // addr_me
    r.take(8)?; // nonce
    let ua_len = r.compact_size()?;
    if ua_len > 256 {
        return None;
    }
    let user_agent = String::from_utf8_lossy(r.take(ua_len as usize)?).into_owned();
    let start_height = i32::from_le_bytes(r.take(4)?.try_into().ok()?);
    Some(Handshake {
        version,
        services,
        user_agent,
        start_height,
    })
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }

    fn compact_size(&mut self) -> Option<u64> {
        let first = self.take(1)?[0];
        Some(match first {
            0xfd => u16::from_le_bytes(self.take(2)?.try_into().ok()?) as u64,
            0xfe => u32::from_le_bytes(self.take(4)?.try_into().ok()?) as u64,
            0xff => u64::from_le_bytes(self.take(8)?.try_into().ok()?),
            n => n as u64,
        })
    }
}

async fn read_message(s: &mut TcpStream, magic: [u8; 4]) -> Result<(String, Vec<u8>), String> {
    let mut head = [0u8; 24];
    s.read_exact(&mut head)
        .await
        .map_err(|e| format!("closed during the handshake ({e})"))?;
    if head[..4] != magic {
        return Err("answered with a different network's magic".to_string());
    }
    let cmd_end = head[4..16].iter().position(|&b| b == 0).unwrap_or(12);
    let command = String::from_utf8_lossy(&head[4..4 + cmd_end]).into_owned();
    let len = u32::from_le_bytes(head[16..20].try_into().unwrap_or_default());
    if len > MAX_PAYLOAD {
        return Err(format!("announced a {len}-byte {command}"));
    }
    let mut payload = vec![0u8; len as usize];
    s.read_exact(&mut payload)
        .await
        .map_err(|e| format!("closed inside {command} ({e})"))?;
    if checksum(&payload) != head[20..24] {
        return Err(format!("{command} failed its checksum"));
    }
    Ok((command, payload))
}

/// Handshake with `addr` ("host:port"), all of it inside `timeout`.
pub async fn probe(addr: &str, timeout: Duration, now: i64) -> Result<Handshake, String> {
    tokio::time::timeout(timeout, probe_inner(addr, MAINNET_MAGIC, now))
        .await
        .unwrap_or_else(|_| {
            Err(format!(
                "no version and verack within {} s",
                timeout.as_secs()
            ))
        })
}

async fn probe_inner(addr: &str, magic: [u8; 4], now: i64) -> Result<Handshake, String> {
    use rand_core::RngCore;
    let mut s = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("TCP connect failed ({e})"))?;
    let nonce = rand_core::OsRng.next_u64();
    let ua = format!("/btx-sentinel:{}/", env!("CARGO_PKG_VERSION"));
    s.write_all(&frame(magic, "version", &version_payload(now, nonce, &ua)))
        .await
        .map_err(|e| format!("could not send version ({e})"))?;
    let mut theirs: Option<Handshake> = None;
    let mut verack = false;
    for _ in 0..MAX_MESSAGES {
        let (cmd, payload) = read_message(&mut s, magic).await?;
        match cmd.as_str() {
            "version" => {
                let v = parse_version(&payload)
                    .ok_or_else(|| "sent a version that does not parse".to_string())?;
                theirs = Some(v);
                // The one reply a handshake owes. Nothing else is ever sent.
                let _ = s.write_all(&frame(magic, "verack", &[])).await;
            }
            "verack" => verack = true,
            _ => {}
        }
        if let (Some(v), true) = (&theirs, verack) {
            let _ = s.shutdown().await;
            return Ok(v.clone());
        }
    }
    Err("no version and verack among the first messages".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// The empty payload's checksum is the one every Bitcoin-derived verack
    /// carries, `5df6e0e2`: the header layout and the double SHA-256 agree.
    #[test]
    fn a_verack_is_framed_like_the_engine_frames_it() {
        let f = frame(MAINNET_MAGIC, "verack", &[]);
        assert_eq!(
            crate::operators::hex(&f),
            "b754580176657261636b000000000000000000005df6e0e2"
        );
    }

    #[test]
    fn our_version_parses_back() {
        let p = version_payload(1_791_000_000, 7, "/btx-sentinel:0.1.0/");
        let v = parse_version(&p).unwrap();
        assert_eq!(v.version, 800_002);
        assert_eq!(v.services, 0);
        assert_eq!(v.user_agent, "/btx-sentinel:0.1.0/");
        assert_eq!(v.start_height, 0);
        assert_eq!(p.len(), 4 + 8 + 8 + 26 + 26 + 8 + 1 + 20 + 4 + 1);
        assert!(parse_version(&p[..40]).is_none());
    }

    /// A fake peer on loopback that speaks the engine's order: version,
    /// a few feature messages, verack. The probe must come back with what the
    /// peer said, and must have sent nothing but version and verack.
    #[tokio::test]
    async fn the_probe_reads_version_and_verack_and_sends_nothing_else() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let peer = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut theirs = version_payload(0, 1, "/BTX:0.34.12/");
            // Services NETWORK|WITNESS, start height 237194.
            theirs[4..12].copy_from_slice(&9u64.to_le_bytes());
            let n = theirs.len();
            theirs[n - 5..n - 1].copy_from_slice(&237_194i32.to_le_bytes());
            let mut sent = Vec::new();
            sent.extend(frame(MAINNET_MAGIC, "version", &theirs));
            sent.extend(frame(MAINNET_MAGIC, "wtxidrelay", &[]));
            sent.extend(frame(MAINNET_MAGIC, "sendaddrv2", &[]));
            sent.extend(frame(MAINNET_MAGIC, "verack", &[]));
            s.write_all(&sent).await.unwrap();
            let mut got = Vec::new();
            let _ = s.read_to_end(&mut got).await;
            got
        });
        let v = probe(&addr, Duration::from_secs(5), 1_791_000_000)
            .await
            .unwrap();
        assert_eq!(v.user_agent, "/BTX:0.34.12/");
        assert_eq!(v.services, 9);
        assert_eq!(v.start_height, 237_194);

        let got = peer.await.unwrap();
        let mut cmds = Vec::new();
        let mut at = 0;
        while at + 24 <= got.len() {
            let end = got[at + 4..at + 16].iter().position(|&b| b == 0).unwrap();
            cmds.push(String::from_utf8_lossy(&got[at + 4..at + 4 + end]).into_owned());
            let len = u32::from_le_bytes(got[at + 16..at + 20].try_into().unwrap()) as usize;
            at += 24 + len;
        }
        assert_eq!(cmds, vec!["version", "verack"]);
    }

    #[tokio::test]
    async fn a_peer_that_never_answers_times_out() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let _hold = tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
            drop(s);
        });
        let err = probe(&addr, Duration::from_millis(300), 0)
            .await
            .unwrap_err();
        assert!(err.contains("no version and verack"), "{err}");
    }

    #[tokio::test]
    async fn a_peer_on_another_network_is_a_failure() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let v = version_payload(0, 1, "/Satoshi:27.0.0/");
            s.write_all(&frame([0xf9, 0xbe, 0xb4, 0xd9], "version", &v))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let err = probe(&addr, Duration::from_secs(5), 0).await.unwrap_err();
        assert!(err.contains("magic"), "{err}");
    }
}
