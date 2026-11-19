// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Local single-viewer UDP recorder for the Rust region's Ferrobot scenario.
//! The advertised loopback socket forwards bytes unchanged to the region's
//! separate loopback address. C# scenarios use their existing capture patch.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ferroan_capture::{Kind, Record, read_complete_prefix, write_capture};
use ferroan_protocol::msgs::UseCircuitCode;
use ferroan_protocol::{Msg, parse_packet};
use tokio::net::UdpSocket;

const MAX_CAPTURE: u64 = 64 * 1024 * 1024;
const MAX_DATAGRAM: usize = 4096;
// Match the FERROCAP1 reader's record bound, including stream appends.
const MAX_RECORDS: usize = 1_000_000;

pub(crate) fn http_records(paths: &[PathBuf]) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    let mut remaining = MAX_CAPTURE;
    for path in paths {
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|file| file.take(remaining + 1).read_to_end(&mut bytes))
            .map_err(|error| format!("read HTTP capture: {error}"))?;
        if bytes.len() as u64 > remaining {
            return Err("HTTP captures exceed 64 MiB".to_string());
        }
        remaining -= bytes.len() as u64;
        for record in read_complete_prefix(&bytes).map_err(|error| error.to_string())? {
            if matches!(record.kind, Kind::HttpRequest | Kind::HttpResponse) {
                if records.len() == MAX_RECORDS {
                    return Err("HTTP captures exceed the record limit".to_string());
                }
                records.push(record);
            }
        }
    }
    Ok(records)
}

pub(crate) async fn start_udp(
    listen: SocketAddr,
    upstream: SocketAddr,
    path: &Path,
) -> Result<(tokio::task::JoinHandle<Result<(), String>>, SocketAddr), String> {
    let front = UdpSocket::bind(listen)
        .await
        .map_err(|error| format!("bind UDP capture: {error}"))?;
    // Port 0 is assigned here. Callers must use this address, not a port
    // observed and then released before the bind.
    let bound = front
        .local_addr()
        .map_err(|error| format!("UDP capture address: {error}"))?;
    let backend = UdpSocket::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    backend
        .connect(upstream)
        .await
        .map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A previous run may have created this path with broader permissions.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    let header = write_capture(&[]).map_err(|error| error.to_string())?;
    file.write_all(&header).map_err(|error| error.to_string())?;
    let owner = crate::live::ESTATE_OWNER
        .parse::<uuid::Uuid>()
        .map_err(|_| "invalid harness owner UUID")?;
    Ok((
        tokio::spawn(async move {
            let mut peer = None;
            let mut incoming = [0u8; MAX_DATAGRAM + 1];
            let mut outgoing = [0u8; MAX_DATAGRAM + 1];
            let mut written = header.len() as u64;
            let mut count = 0;
            loop {
                tokio::select! {
                    received = front.recv_from(&mut incoming) => {
                        let (len, from) = received.map_err(|error| format!("UDP capture receive: {error}"))?;
                        if len == 0 || len > MAX_DATAGRAM || peer.is_some_and(|peer| peer != from) { continue; }
                        if peer.is_none() {
                            let Ok(packet) = parse_packet(&incoming[..len]) else { continue; };
                            if packet.id != UseCircuitCode::ID || !packet.decode::<UseCircuitCode>().is_ok_and(|code| code.circuit_code.id == owner) { continue; }
                            peer = Some(from);
                        }
                        record(&mut file, &mut written, &mut count, header.len(), Kind::UdpIn, &incoming[..len])?;
                        backend.send(&incoming[..len]).await.map_err(|error| format!("UDP capture forward: {error}"))?;
                    }
                    received = backend.recv(&mut outgoing) => {
                        let len = received.map_err(|error| format!("UDP capture reply: {error}"))?;
                        if len == 0 || len > MAX_DATAGRAM { continue; }
                        if let Some(peer) = peer {
                            record(&mut file, &mut written, &mut count, header.len(), Kind::UdpOut, &outgoing[..len])?;
                            front.send_to(&outgoing[..len], peer).await.map_err(|error| format!("UDP capture return: {error}"))?;
                        }
                    }
                }
            }
        }),
        bound,
    ))
}

fn record(
    file: &mut File,
    written: &mut u64,
    count: &mut usize,
    header_len: usize,
    kind: Kind,
    bytes: &[u8],
) -> Result<(), String> {
    if *count == MAX_RECORDS {
        return Err("UDP capture exceeds the record limit".to_string());
    }
    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "capture clock precedes epoch")?
        .as_millis() as u64;
    let encoded = write_capture(&[Record {
        kind,
        time_ms,
        bytes: bytes.to_vec(),
    }])
    .map_err(|error| error.to_string())?;
    // write_capture always starts with the header emitted when the file opened.
    let record = &encoded[header_len..];
    if *written + record.len() as u64 > MAX_CAPTURE {
        return Err("UDP capture exceeds 64 MiB".to_string());
    }
    file.write_all(record).map_err(|error| error.to_string())?;
    *written += record.len() as u64;
    *count += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroan_capture::read_capture;
    use ferroan_protocol::build_packet;
    use ferroan_protocol::msgs::{CompletePingCheck, StartPingCheck};
    use std::time::Duration;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let root = crate::repo_root()
                .join(".scratch")
                .join(format!("udp-capture-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn wire<M: Msg>(message: &M) -> Vec<u8> {
        let mut bytes = Vec::new();
        build_packet(message, 1, false, false, &[], &mut bytes);
        bytes
    }
    async fn receive(socket: &UdpSocket) -> Vec<u8> {
        let mut bytes = [0; MAX_DATAGRAM + 1];
        let len = tokio::time::timeout(Duration::from_secs(2), socket.recv(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        bytes[..len].to_vec()
    }

    #[tokio::test]
    async fn relay_records_exact_bytes_only_for_the_selected_peer() {
        let scratch = Scratch::new();
        let path = scratch.0.join("udp.cap");
        std::fs::write(&path, b"old capture").unwrap();
        let upstream = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (task, listen) = start_udp(
            "127.0.0.1:0".parse().unwrap(),
            upstream.local_addr().unwrap(),
            &path,
        )
        .await
        .unwrap();
        let owner = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let stranger = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        owner.connect(listen).await.unwrap();
        stranger.connect(listen).await.unwrap();
        let mut code = UseCircuitCode::default();
        code.circuit_code.id = uuid::Uuid::from_u128(9);
        stranger.send(&wire(&code)).await.unwrap();
        code.circuit_code.id = crate::live::ESTATE_OWNER.parse().unwrap();
        let open = wire(&code);
        owner.send(&open).await.unwrap();
        let mut buffer = [0; MAX_DATAGRAM + 1];
        let (len, relay) =
            tokio::time::timeout(Duration::from_secs(2), upstream.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(&buffer[..len], open);
        upstream.connect(relay).await.unwrap();
        stranger.send(&open).await.unwrap();
        owner.send(&[]).await.unwrap();
        owner.send(&[0; MAX_DATAGRAM + 1]).await.unwrap();
        let ping = wire(&StartPingCheck::default());
        owner.send(&ping).await.unwrap();
        assert_eq!(receive(&upstream).await, ping);
        upstream.send(&[]).await.unwrap();
        upstream.send(&[0; MAX_DATAGRAM + 1]).await.unwrap();
        let reply = wire(&CompletePingCheck::default());
        upstream.send(&reply).await.unwrap();
        assert_eq!(receive(&owner).await, reply);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let records = read_capture(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!((&records[0].kind, &records[0].bytes), (&Kind::UdpIn, &open));
        assert_eq!((&records[1].kind, &records[1].bytes), (&Kind::UdpIn, &ping));
        assert_eq!(
            (&records[2].kind, &records[2].bytes),
            (&Kind::UdpOut, &reply)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn append_refuses_byte_and_record_limits_before_writing() {
        let scratch = Scratch::new();
        let path = scratch.0.join("limits.cap");
        let mut file = File::create(&path).unwrap();
        let header = write_capture(&[]).unwrap();
        file.write_all(&header).unwrap();
        let mut written = header.len() as u64;
        let mut count = MAX_RECORDS;
        assert!(
            record(
                &mut file,
                &mut written,
                &mut count,
                header.len(),
                Kind::UdpIn,
                &[1]
            )
            .unwrap_err()
            .contains("record limit")
        );
        count = 0;
        written = MAX_CAPTURE;
        assert!(
            record(
                &mut file,
                &mut written,
                &mut count,
                header.len(),
                Kind::UdpIn,
                &[1]
            )
            .unwrap_err()
            .contains("64 MiB")
        );
        assert_eq!(std::fs::read(path).unwrap(), header);
    }
}
