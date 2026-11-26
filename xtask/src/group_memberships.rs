// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(not(target_os = "linux"))]
pub(crate) fn run() -> Result<String, String> {
    Err("the groups live comparison requires the Linux live build".into())
}

#[cfg(target_os = "linux")]
pub(crate) fn run() -> Result<String, String> {
    use crate::{
        assets, live,
        live_proxy::Proxies,
        ports::{self, Slot},
    };
    use ferroan_capture::Kind;
    use ferroan_config::Config;
    use ferroan_region::groups::GroupsClient;
    use std::{
        collections::BTreeMap,
        fs,
        net::{Ipv4Addr, SocketAddr},
        process::Command,
    };
    use uuid::Uuid;

    let _state = crate::grid_state::claim()?;
    let fixture = crate::repo_root().join("fixtures");
    let run = live::scratch_run().join(format!("groups-{}", Uuid::new_v4()));
    let empty = run.join("empty");
    fs::create_dir_all(&empty).map_err(|e| e.to_string())?;
    let db = run.join("groups.db");
    fs::copy(fixture.join("db/sqlite/os_groups_Store.db"), &db).map_err(|e| e.to_string())?;
    let seed =
        fs::File::open(fixture.join("group-memberships/seed.sql")).map_err(|e| e.to_string())?;
    if !Command::new("sqlite3")
        .arg("-bail")
        .arg(&db)
        .stdin(seed)
        .status()
        .map_err(|e| e.to_string())?
        .success()
    {
        return Err("could not seed the parcel membership store".into());
    }
    let port = ports::alloc(Slot::RobustPrivate)?;
    let ini = run.join("robust.ini");
    let content = format!(
        "[Const]\nBaseHostname=127.0.0.1\nPrivatePort={port}\nPublicPort={port}\n[Startup]\nRegistryLocation=.\nConsoleHistoryFileEnabled=false\nEnableRobustSelfsignedCertSupport=false\n[Network]\nport={port}\nbind_ip_address=127.0.0.1\n[ServiceList]\nGroupsServiceConnector={port}/OpenSim.Addons.Groups.dll:GroupsServiceRobustConnector\n[DatabaseService]\nStorageProvider=OpenSim.Data.SQLite.dll\nConnectionString=\"URI=file:{}\"\n",
        db.display()
    );
    live::write_secret(&ini, &content)?;
    let bin = live::grid_server_binary();
    let log = run.join("robust.log");
    let mut server = assets::spawn_owned(
        &bin,
        &[
            "--inimaster",
            ini.to_str().ok_or("invalid ini path")?,
            "--inidirectory",
            empty.to_str().ok_or("invalid config path")?,
            "--console",
            "basic",
        ],
        bin.parent().ok_or("missing live build")?,
        &log,
        true,
        true,
    )?;
    assets::wait_service(&mut server, "groups", port, &log)?;
    let address = |port| SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let source = |port| {
        let config = Config::parse(&format!("[Groups]\nEnabled=true\nModule=Groups Module V2\nServicesConnectorModule=Groups Remote Service Connector\nGroupsServerURI=http://127.0.0.1:{port}/\n")).map_err(|e|e.to_string())?;
        GroupsClient::from_config(&config).map_err(|e| e.to_string())
    };
    let capture = run.join("robust.cap");
    let (front, proxies) = Proxies::start(&[(
        address(ports::alloc(Slot::PrivateProxy)?),
        address(port),
        capture.clone(),
    )])?;
    let direct = source(port)?;
    let proxied = source(front.first().ok_or("missing groups proxy")?.port())?;
    let cases = [
        (
            "member",
            1,
            BTreeMap::from([(Uuid::from_u128(10), 33554440), (Uuid::from_u128(11), 0)]),
        ),
        ("nonmember", 3, BTreeMap::new()),
        (
            "owner",
            2,
            BTreeMap::from([(Uuid::from_u128(10), u64::MAX)]),
        ),
    ];
    proxies.block_on(async {
        // Robust binds before its handlers are ready.
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if direct
                    .memberships(Uuid::from_u128(2))
                    .await
                    .is_ok_and(|rows| rows.get(&Uuid::from_u128(10)) == Some(&u64::MAX))
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| "groups route did not become ready")?;
        for (name, agent, expected) in &cases {
            let actual = proxied
                .memberships(Uuid::from_u128(*agent))
                .await
                .map_err(|e| e.to_string())?;
            if &actual != expected {
                return Err(format!(
                    "{name}: expected {expected:?}, received {actual:?}"
                ));
            }
        }
        Ok::<_, String>(())
    })?;
    proxies.finish()?;
    let records = crate::grid::ferrobot_capture::http_records(&[capture])?;
    if records.len() != cases.len() * 2 {
        return Err("unexpected groups exchange count".into());
    }
    for ((name, agent, _), pair) in cases.iter().zip(records.as_chunks::<2>().0) {
        let [request, response] = pair;
        let body = |bytes: &[u8]| {
            bytes
                .windows(4)
                .position(|b| b == b"\r\n\r\n")
                .map(|i| i + 4)
                .ok_or("missing HTTP headers")
        };
        let expected = format!(
            "AgentID={}&ALL=true&RequestingAgentID={}&METHOD=GETMEMBERSHIP",
            Uuid::from_u128(*agent),
            Uuid::nil()
        );
        if request.kind != Kind::HttpRequest
            || !request.bytes.starts_with(b"POST /groups HTTP/")
            || request.bytes[body(&request.bytes)?..] != *expected.as_bytes()
            || response.kind != Kind::HttpResponse
        {
            return Err(format!("{name}: unexpected groups request or response"));
        }
        let xml = &response.bytes[body(&response.bytes)?..];
        ferroan_http::parse_xml_response(std::str::from_utf8(xml).map_err(|_| "invalid UTF-8")?)
            .map_err(|e| e.to_string())?;
        fs::write(run.join(format!("{name}.xml")), xml).map_err(|e| e.to_string())?;
    }
    Ok(format!(
        "groups: member, nonmember and owner match Pin8; replies in {}",
        run.display()
    ))
}
