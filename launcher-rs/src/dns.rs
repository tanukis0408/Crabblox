use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, Mutex};

pub const ROBLOX_EDGE_IPS: [&str; 3] = ["128.116.5.3", "128.116.21.3", "128.116.115.3"];

struct CacheEntry {
    expires_at: Instant,
    response: Vec<u8>,
}

pub struct DnsForwarder {
    pub port: u16,
    shutdown_tx: broadcast::Sender<()>,
}

impl DnsForwarder {
    pub async fn start() -> anyhow::Result<Self> {
        let socket = UdpSocket::bind("127.0.0.1:0").await?;
        let port = socket.local_addr()?.port();
        let (shutdown_tx, _) = broadcast::channel(1);

        let cache = Arc::new(Mutex::new(HashMap::<Vec<u8>, CacheEntry>::new()));
        let socket = Arc::new(socket);
        let mut shutdown_rx = shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                    recv_res = socket.recv_from(&mut buf) => {
                        let (len, peer) = match recv_res {
                            Ok(res) => res,
                            Err(_) => {
                                tokio::time::sleep(Duration::from_millis(10)).await;
                                continue;
                            }
                        };
                        if len < 12 {
                            continue;
                        }

                        let query = buf[..len].to_vec();
                        let socket_clone = socket.clone();
                        let cache_clone = cache.clone();

                        tokio::spawn(async move {
                            handle_query(query, peer, socket_clone, cache_clone).await;
                        });
                    }
                }
            }
        });

        tokio::spawn(prewarm_dns());

        Ok(Self { port, shutdown_tx })
    }

    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

impl Drop for DnsForwarder {
    fn drop(&mut self) {
        self.stop();
    }
}

pub async fn prewarm_dns() {
    let domains = [
        "roblox.com",
        "www.roblox.com",
        "users.roblox.com",
        "auth.roblox.com",
        "setup.rbxcdn.com",
        "assetdelivery.roblox.com",
        "clientsettingscdn.roblox.com",
    ];

    for domain in domains {
        let d = domain.to_string();
        tokio::spawn(async move {
            let _ = tokio::net::lookup_host(format!("{d}:443")).await;
        });
    }
}

async fn handle_query(
    query: Vec<u8>,
    peer: SocketAddr,
    socket: Arc<UdpSocket>,
    cache: Arc<Mutex<HashMap<Vec<u8>, CacheEntry>>>,
) {
    let (qname, qtype) = extract_qname_and_type(&query);

    // Direct routing for auth.roblox.com to avoid TSPU / Russian Cloudflare blocks
    if qname == "auth.roblox.com" {
        let resp = if qtype == 1 {
            make_a_response(&query, &ROBLOX_EDGE_IPS)
        } else {
            make_empty_response(&query)
        };
        let _ = socket.send_to(&resp, peer).await;
        return;
    }

    // Direct routing for tr.rbxcdn.com (thumbnails/assets)
    // Roblox uses a broken intermediate CNAME chain on some public DNS (tr -> trns1 -> traws -> cloudfront)
    if qname == "tr.rbxcdn.com" {
        if qtype == 1 {
            let mut ips: Vec<String> = Vec::new();
            if let Ok(addrs) = tokio::net::lookup_host("traws.rbxcdn.com:443").await {
                for addr in addrs {
                    if let SocketAddr::V4(v4) = addr {
                        ips.push(v4.ip().to_string());
                    }
                }
            }
            if ips.is_empty() {
                ips = vec![
                    "143.204.238.14".into(),
                    "143.204.238.81".into(),
                    "143.204.238.74".into(),
                    "143.204.238.88".into(),
                ];
            }
            let ip_slices: Vec<&str> = ips.iter().map(|s| s.as_str()).collect();
            let resp = make_a_response(&query, &ip_slices);
            let _ = socket.send_to(&resp, peer).await;
            return;
        } else {
            let resp = make_empty_response(&query);
            let _ = socket.send_to(&resp, peer).await;
            return;
        }
    }

    let key = query[2..].to_vec();
    let now = Instant::now();

    {
        let cache_lock = cache.lock().await;
        if let Some(entry) = cache_lock.get(&key) {
            if entry.expires_at > now {
                let mut resp = query[..2].to_vec();
                resp.extend_from_slice(&entry.response);
                let _ = socket.send_to(&resp, peer).await;
                return;
            }
        }
    }

    // Resolve via DNS-over-HTTPS (Google or Quad9)
    let upstream_resp = resolve_doh(&query).await;
    if let Some(raw_resp) = upstream_resp {
        let mut sanitized = sanitize_response(&raw_resp);
        if sanitized.len() >= 2 && query.len() >= 2 {
            sanitized[0..2].copy_from_slice(&query[0..2]);
        }
        let answers_count = if sanitized.len() >= 8 {
            u16::from_be_bytes([sanitized[6], sanitized[7]])
        } else {
            0
        };
        if answers_count > 0 {
            {
                let mut cache_lock = cache.lock().await;
                cache_lock.insert(
                    key,
                    CacheEntry {
                        expires_at: now + Duration::from_secs(60),
                        response: sanitized[2..].to_vec(),
                    },
                );
            }
            let _ = socket.send_to(&sanitized, peer).await;
            return;
        }
    }

    // Fallback: Resolve via direct host system socket resolver if DoH failed, timed out, or returned no answers
    if qtype == 1 && !qname.is_empty() {
        let host_lookup = match tokio::net::lookup_host(format!("{qname}:443")).await {
            Ok(addrs) => Ok(addrs),
            Err(_) => tokio::net::lookup_host(format!("{qname}:80")).await,
        };

        if let Ok(addrs) = host_lookup {
            let mut ips = Vec::new();
            for addr in addrs {
                if let SocketAddr::V4(v4) = addr {
                    let octets = v4.ip().octets();
                    let is_cf = (octets[0] == 104 && (16..=31).contains(&octets[1]))
                        || (octets[0] == 172 && (64..=71).contains(&octets[1]));
                    if is_cf {
                        ips.push("128.116.5.3".to_string());
                    } else {
                        ips.push(v4.ip().to_string());
                    }
                }
            }
            if !ips.is_empty() {
                let ip_slices: Vec<&str> = ips.iter().map(|s| s.as_str()).collect();
                let resp = make_a_response(&query, &ip_slices);
                {
                    let mut cache_lock = cache.lock().await;
                    cache_lock.insert(
                        key,
                        CacheEntry {
                            expires_at: now + Duration::from_secs(60),
                            response: resp[2..].to_vec(),
                        },
                    );
                }
                let _ = socket.send_to(&resp, peer).await;
                return;
            }
        }
    }

    // Cleanly respond with empty response if resolution failed or non-A type, so client doesn't hang
    let resp = make_empty_response(&query);
    let _ = socket.send_to(&resp, peer).await;
}

fn extract_qname_and_type(query: &[u8]) -> (String, u16) {
    if query.len() < 12 {
        return (String::new(), 1);
    }
    let mut offset = 12;
    let mut labels = Vec::new();

    while offset < query.len() {
        let len = query[offset] as usize;
        if len == 0 {
            offset += 1;
            break;
        }
        offset += 1;
        if offset + len > query.len() {
            return (String::new(), 1);
        }
        if let Ok(label) = std::str::from_utf8(&query[offset..offset + len]) {
            labels.push(label.to_lowercase());
        }
        offset += len;
    }

    let qname = labels.join(".");
    let qtype = if offset + 4 <= query.len() {
        u16::from_be_bytes([query[offset], query[offset + 1]])
    } else {
        1
    };

    (qname, qtype)
}

fn make_a_response(query: &[u8], ips: &[&str]) -> Vec<u8> {
    let mut resp = Vec::new();
    resp.extend_from_slice(&query[..2]); // tx id
    resp.extend_from_slice(&[0x81, 0x80]); // response, no error
    resp.extend_from_slice(&1u16.to_be_bytes()); // questions: 1
    resp.extend_from_slice(&(ips.len() as u16).to_be_bytes()); // answers
    resp.extend_from_slice(&[0, 0, 0, 0]); // auth: 0, add: 0

    // Question section
    let mut offset = 12;
    while offset < query.len() {
        let b = query[offset];
        if b == 0 {
            offset += 1;
            break;
        }
        if (b & 0xc0) == 0xc0 {
            offset += 2;
            break;
        }
        offset += 1 + (b as usize);
    }
    offset = (offset + 4).min(query.len());
    resp.extend_from_slice(&query[12..offset]);

    // Answers
    for ip_str in ips {
        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
            resp.extend_from_slice(&[0xc0, 0x0c]); // name pointer
            resp.extend_from_slice(&1u16.to_be_bytes()); // type A
            resp.extend_from_slice(&1u16.to_be_bytes()); // class IN
            resp.extend_from_slice(&300u32.to_be_bytes()); // TTL 300s
            resp.extend_from_slice(&4u16.to_be_bytes()); // len 4
            resp.extend_from_slice(&ip.octets());
        }
    }

    resp
}

fn make_empty_response(query: &[u8]) -> Vec<u8> {
    let mut resp = Vec::new();
    resp.extend_from_slice(&query[..2]);
    resp.extend_from_slice(&[0x81, 0x80]);
    resp.extend_from_slice(&1u16.to_be_bytes()); // questions: 1
    resp.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // answers: 0

    let mut offset = 12;
    while offset < query.len() {
        let b = query[offset];
        if b == 0 {
            offset += 1;
            break;
        }
        if (b & 0xc0) == 0xc0 {
            offset += 2;
            break;
        }
        offset += 1 + (b as usize);
    }
    offset = (offset + 4).min(query.len());
    resp.extend_from_slice(&query[12..offset]);
    resp
}

async fn resolve_doh(query: &[u8]) -> Option<Vec<u8>> {
    let query_bytes = query.to_vec();
    tokio::task::spawn_blocking(move || {
        let doh_endpoints = [
            "https://dns.google/dns-query",
            "https://cloudflare-dns.com/dns-query",
            "https://dns.quad9.net/dns-query",
        ];
        for url in doh_endpoints {
            let resp = ureq::post(url)
                .set("Content-Type", "application/dns-message")
                .set("Accept", "application/dns-message")
                .timeout(Duration::from_secs(2))
                .send_bytes(&query_bytes);

            match resp {
                Ok(response) => {
                    if response.status() == 200 {
                        use std::io::Read;
                        let mut reader = response.into_reader().take(65536);
                        let mut out = Vec::new();
                        if std::io::copy(&mut reader, &mut out).is_ok() && out.len() >= 12 {
                            return Some(out);
                        }
                    }
                }
                Err(_) => {
                    // Try next DoH provider if this one times out or errors
                    continue;
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

fn sanitize_response(response: &[u8]) -> Vec<u8> {
    if response.len() < 12 {
        return response.to_vec();
    }
    let mut res = response.to_vec();
    let answers = u16::from_be_bytes([response[6], response[7]]) as usize;

    fn skip_name(data: &[u8], mut pos: usize) -> Option<usize> {
        let mut jumps = 0;
        while pos < data.len() {
            jumps += 1;
            if jumps > 128 {
                return None;
            }
            let b = data[pos];
            if b == 0 {
                return Some(pos + 1);
            }
            if (b & 0xc0) == 0xc0 {
                return Some(pos + 2);
            }
            pos += 1 + (b as usize);
        }
        None
    }

    let mut offset = match skip_name(&res, 12) {
        Some(o) => o + 4,
        None => return res,
    };

    for _ in 0..answers {
        offset = match skip_name(&res, offset) {
            Some(o) => o,
            None => break,
        };
        if offset + 10 > res.len() {
            break;
        }
        let qtype = u16::from_be_bytes([res[offset], res[offset + 1]]);
        let dlen = u16::from_be_bytes([res[offset + 8], res[offset + 9]]) as usize;
        offset += 10;

        if qtype == 1 && dlen == 4 && offset + 4 <= res.len() {
            let ip = Ipv4Addr::new(res[offset], res[offset + 1], res[offset + 2], res[offset + 3]);
            let octets = ip.octets();
            // Check for Cloudflare ranges: 104.16.0.0 - 104.31.255.255, 172.64.0.0 - 172.71.255.255
            let is_cf = (octets[0] == 104 && (16..=31).contains(&octets[1]))
                || (octets[0] == 172 && (64..=71).contains(&octets[1]));
            if is_cf {
                // Replace with Roblox edge gateway
                res[offset..offset + 4].copy_from_slice(&[128, 116, 5, 3]);
            }
        }
        offset += dlen;
    }

    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dns_forwarder_lifecycle() {
        let forwarder = DnsForwarder::start().await.expect("Failed to start forwarder");
        assert!(forwarder.port > 0);
        forwarder.stop();
    }

    #[tokio::test]
    async fn test_resolve_doh_or_fallback() {
        // Query for roblox.com A record
        let query = [
            0x12, 0x34, // ID
            0x01, 0x00, // Standard query
            0x00, 0x01, // Questions: 1
            0x00, 0x00, // Answers: 0
            0x00, 0x00, // Authority: 0
            0x00, 0x00, // Additional: 0
            0x06, b'r', b'o', b'b', b'l', b'o', b'x',
            0x03, b'c', b'o', b'm',
            0x00,       // null terminator
            0x00, 0x01, // Type A
            0x00, 0x01, // Class IN
        ];

        let resp = resolve_doh(&query).await;
        // Either DoH succeeds or times out cleanly
        if let Some(data) = resp {
            assert!(data.len() >= 12);
        }
    }

    #[test]
    fn test_make_responses() {
        let query = [
            0xaa, 0xbb, // tx id
            0x01, 0x00,
            0x00, 0x01,
            0x00, 0x00,
            0x00, 0x00,
            0x00, 0x00,
            0x04, b't', b'e', b's', b't',
            0x00,
            0x00, 0x01,
            0x00, 0x01,
        ];
        let empty = make_empty_response(&query);
        assert_eq!(&empty[..2], &[0xaa, 0xbb]);

        let a_resp = make_a_response(&query, &["1.2.3.4"]);
        assert_eq!(&a_resp[..2], &[0xaa, 0xbb]);
        assert!(a_resp.len() > query.len());
    }
}

