// Copyright (c) Contributors, http://opensimulator.org/
// See CONTRIBUTORS.TXT for a full list of copyright holders.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//     * Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//     * Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in the
//       documentation and/or other materials provided with the distribution.
//     * Neither the name of the OpenSimulator Project nor the
//       names of its contributors may be used to endorse or promote products
//       derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE DEVELOPERS ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//
// Ported from Source/OpenSim.Region.CoreModules/Framework/EntityTransfer/EntityTransferModule.cs
// and Source/OpenSim.Services.Connectors/Grid/GridServicesConnector.cs.
// KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{GridClient, RegionConfig, endpoint_for, located_host, parse_located, service_form};
use crate::simulation_client::RegionEnd;
use ferroan_http::{Field, FieldMap};
use std::{net::SocketAddr, time::Duration};
use tokio::time::Instant;
use uuid::Uuid;

/// The scene's 30-second grid range cache. Each root filters the same candidates
/// with its own position and effective RegionViewDistance.
pub struct VisibleNeighbours {
    grid: GridClient,
    scope: Uuid,
    home: Uuid,
    origin: [u32; 2],
    size: [u32; 2],
    distances: crate::config::RegionViewDistances,
    cached: Option<(Instant, Vec<VisibleNeighbour>)>,
}

#[derive(Clone, Debug)]
pub struct VisibleNeighbour {
    pub region: RegionEnd,
    pub size: [u32; 2],
    host: String,
    port: u16,
}

impl VisibleNeighbour {
    fn from_row(field: &Field, home: Uuid) -> Option<Self> {
        let Field::Map(row) = field else { return None };
        // Pre-0.8.1 simulators omit flags. RegionOnline is bit 2.
        if let Some(flags) = row.get("flags") {
            let flags = match flags {
                Field::Str(value) => value.parse::<i32>().ok()?,
                Field::Int(value) => *value,
                _ => return None,
            };
            if flags & 4 == 0 {
                return None;
            }
        }
        let r = parse_located(row)?;
        if r.region_id == home {
            return None;
        }
        Some(Self {
            host: located_host(row)?,
            port: r.endpoint.port(),
            size: [r.size_x, r.size_y],
            region: RegionEnd {
                region_id: r.region_id,
                name: r.name,
                server_uri: r.server_uri,
                loc_x: r.loc_x,
                loc_y: r.loc_y,
            },
        })
    }

    fn in_view(&self, center: [i32; 2], distance: i32) -> bool {
        let intersects = |center: i32, start: u32, size: u32| {
            center.wrapping_add(distance) >= start as i32
                && center.wrapping_sub(distance) <= start.wrapping_add(size) as i32
        };
        intersects(center[0], self.region.loc_x, self.size[0])
            && intersects(center[1], self.region.loc_y, self.size[1])
    }

    /// Resolve only when opening a new circuit. Retained children need no
    /// new viewer endpoint for a position PUT.
    pub async fn viewer_endpoint(&self) -> Option<SocketAddr> {
        let addr = endpoint_for(&self.host, self.port).await;
        (!addr.ip().is_unspecified() && addr.port() != 0).then_some(addr)
    }
}

impl VisibleNeighbours {
    pub fn new(grid: GridClient, home: &RegionConfig) -> Self {
        Self {
            grid,
            scope: home.scope_id,
            home: home.id,
            origin: home.location.map(|tile| tile.wrapping_mul(256)),
            size: home.size,
            distances: home.region_view,
            cached: None,
        }
    }

    /// Pass the presence's draw distance, also sent unchanged in its circuit and PUT.
    pub async fn in_view(
        &mut self,
        local_position: [f32; 3],
        draw_distance: f32,
    ) -> Vec<VisibleNeighbour> {
        let distance = self.distances.for_draw_distance(draw_distance) as i32;
        if distance <= 0 || self.distances.maximum as u32 <= 1 {
            return Vec::new();
        }
        let candidates = match &self.cached {
            Some((time, rows)) if time.elapsed() < Duration::from_secs(30) => rows,
            _ => {
                let fields = self.range_fields();
                let reply = match service_form(&self.grid.endpoint, &fields).await {
                    Ok(reply) => reply,
                    Err(_) => {
                        // GetRegionRange returns an empty list on failure; GetNeighbors caches it.
                        tracing::warn!("grid range query failed; caching an empty neighbour list");
                        FieldMap::new()
                    }
                };
                let rows = reply
                    .values()
                    .filter_map(|row| VisibleNeighbour::from_row(row, self.home))
                    .collect();
                &self.cached.insert((Instant::now(), rows)).1
            }
        };
        let center = [
            (self.origin[0] as i32).wrapping_add(local_position[0] as i32),
            (self.origin[1] as i32).wrapping_add(local_position[1] as i32),
        ];
        candidates
            .iter()
            .filter(|r| r.in_view(center, distance))
            .cloned()
            .collect()
    }

    fn range_fields(&self) -> FieldMap {
        let margin = self.distances.maximum as u32 - 1;
        let interval = |axis: usize| {
            let start = self.origin[axis];
            (
                start.wrapping_sub(margin) as i32,
                start.wrapping_add(self.size[axis]).wrapping_add(margin) as i32,
            )
        };
        let (xmin, xmax) = interval(0);
        let (ymin, ymax) = interval(1);
        FieldMap::from([
            ("SCOPEID".into(), Field::Str(self.scope.to_string())),
            ("XMIN".into(), Field::Str(xmin.to_string())),
            ("XMAX".into(), Field::Str(xmax.to_string())),
            ("YMIN".into(), Field::Str(ymin.to_string())),
            ("YMAX".into(), Field::Str(ymax.to_string())),
            ("METHOD".into(), Field::Str("get_region_range".into())),
        ])
    }
}
