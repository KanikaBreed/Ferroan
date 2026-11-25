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
// Ported to Rust for Ferroan from
//   Source/OpenSim.Region.ClientStack.LindenUDP/TokenBucket.cs
//   Source/OpenSim.Region.ClientStack.LindenUDP/ThrottleRates.cs
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPClient.cs
//     (throttle constructor, SetThrottles, RemoveTokens gate)
//   Source/OpenSim.Region.ClientStack.LindenUDP/LLUDPServer.cs
//     (scene throttle parent)
//   KanikaBreed/OpenSim-Tranquillity @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors,
// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-circuit category token buckets.
//!
//! One scene bucket is the parent of every circuit on a listener. Each circuit
//! has a client bucket and eight category buckets. Adaptive adjustment is off.
//! A send is allowed when the category still has any positive credit; the
//! packet size is then subtracted and the credit may go negative.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::time::Instant;

pub(crate) const RESEND: usize = 0;
pub(crate) const LAND: usize = 1;
pub(crate) const TASK: usize = 4;
const SLOTS: usize = 8;

const MIN_DRIP: f32 = 500.0;
const MAX_BURST: f32 = 7_500.0;
const MIN_FLOW: f32 = 50_000.0;
const BURST_TIME: f32 = 10e-3;
/// `scene_throttle_max_bps` when `[ClientStack.LindenUDP]` exists but omits it.
pub const SCENE_MAX_BPS: i32 = 6_250_000;
/// `ThrottleRates.ClientMaxRate` before configuration.
pub const CLIENT_MAX_BPS: i32 = 640_000;
/// `ThrottleRates` clamps a configured client cap to this range.
pub const CLIENT_MAX_BPS_RANGE: std::ops::RangeInclusive<i32> = 6_250..=1_000_000;
#[cfg(test)]
const SCENE_RATE: f32 = SCENE_MAX_BPS as f32;

/// Resend, Land, Wind, Cloud, Task, Texture, Asset, then the unused eighth slot.
const DEFAULT_RATES: [f32; SLOTS] = [
    6_625.0, 9_125.0, 1_750.0, 1_750.0, 18_500.0, 18_500.0, 10_500.0, 0.0,
];

/// Floors applied to the seven viewer floats, in blob order.
const CHANNEL_FLOOR: [f32; 7] = [1_000.0, 1_000.0, 50.0, 50.0, 1_000.0, 1_000.0, 1_000.0];

/// Assignment order after the blob has been clamped. Asset is stored before Task.
const ASSIGN_ORDER: [usize; 7] = [RESEND, LAND, 2, 3, 6, TASK, 5];

#[derive(Clone, Copy)]
struct Bucket {
    drip_rate: f32,
    total_drip_request: f32,
    requested_burst: f32,
    tokens: f32,
    last_drip: f64,
}

impl Bucket {
    fn at(now: f64) -> Self {
        Self {
            drip_rate: 0.0,
            total_drip_request: 0.0,
            requested_burst: 0.0,
            tokens: 0.0,
            last_drip: now + 1_000.0,
        }
    }

    fn requested_drip(self) -> f32 {
        if self.drip_rate == 0.0 {
            self.total_drip_request
        } else {
            self.drip_rate
        }
    }

    fn set_requested_drip(&mut self, value: f32) {
        let rate = if value < 0.0 { 0.0 } else { value };
        self.drip_rate = rate;
        self.total_drip_request = rate;
    }

    fn set_requested_burst(&mut self, value: f32) {
        let mut rate = if value < 0.0 { 0.0 } else { value };
        if rate > MAX_BURST {
            rate = MAX_BURST;
        }
        self.requested_burst = rate;
    }

    /// Deposit credit for the elapsed interval, then allow the send when any
    /// credit remains. The size is subtracted even when it exceeds that credit.
    fn remove_tokens(&mut self, amount: i32, now: f64, drip_rate: f32, burst: f32) -> bool {
        self.drip(now, drip_rate, burst);
        if self.tokens > 0.0 {
            self.tokens -= amount as f32;
            true
        } else {
            false
        }
    }

    fn drip(&mut self, now: f64, drip_rate: f32, burst: f32) {
        if drip_rate == 0.0 {
            return;
        }
        let delta = now - self.last_drip;
        self.last_drip = now;
        if delta <= 0.0 {
            return;
        }
        self.tokens += (delta as f32) * drip_rate;
        if self.tokens > burst {
            self.tokens = burst;
        }
    }
}

struct SceneState {
    bucket: Bucket,
    children: BTreeMap<u64, f32>,
    next_id: u64,
}

/// `[ClientStack.LindenUDP]` throttle caps, in bytes per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitThrottle {
    /// `client_throttle_max_bps`. Values outside 6250..=1000000 are clamped.
    pub client_max_bps: i32,
    /// `scene_throttle_max_bps`. Zero or negative leaves the scene unlimited.
    pub scene_max_bps: i32,
}

impl Default for CircuitThrottle {
    fn default() -> Self {
        Self {
            client_max_bps: CLIENT_MAX_BPS,
            scene_max_bps: SCENE_MAX_BPS,
        }
    }
}

/// The region parent. Every circuit on one listener shares it.
pub(crate) struct SceneThrottle {
    inner: Mutex<SceneState>,
    /// The cap each client bucket on this listener is opened with.
    client_max: f32,
}

impl SceneThrottle {
    #[cfg(test)]
    pub(crate) fn stock() -> Arc<Self> {
        Self::configured(CircuitThrottle::default())
    }

    pub(crate) fn configured(caps: CircuitThrottle) -> Arc<Self> {
        // The clamp keeps every channel floor (at most 1000) below the cap.
        let client = caps
            .client_max_bps
            .clamp(*CLIENT_MAX_BPS_RANGE.start(), *CLIENT_MAX_BPS_RANGE.end());
        Arc::new(Self::new(caps.scene_max_bps as f32, client as f32, 0.0))
    }

    /// `rate` is `scene_throttle_max_bps`. Zero or below leaves the scene
    /// unlimited: the bucket then reports its children's total as its rate.
    fn new(rate: f32, client_max: f32, now: f64) -> Self {
        let mut bucket = Bucket::at(now);
        bucket.set_requested_drip(rate);
        bucket.set_requested_burst(rate * BURST_TIME);
        Self {
            inner: Mutex::new(SceneState {
                bucket,
                children: BTreeMap::new(),
                next_id: 1,
            }),
            client_max,
        }
    }

    pub(crate) fn client_max(&self) -> f32 {
        self.client_max
    }

    fn lock(&self) -> MutexGuard<'_, SceneState> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn register(&self, request: f32) -> u64 {
        let mut state = self.lock();
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.children.insert(id, request);
        state.bucket.total_drip_request = sum_values(&state.children);
        id
    }

    fn update(&self, id: u64, request: f32) {
        let mut state = self.lock();
        match state.children.entry(id) {
            Entry::Occupied(mut entry) => {
                entry.insert(request);
            }
            Entry::Vacant(_) => return,
        }
        state.bucket.total_drip_request = sum_values(&state.children);
    }

    fn unregister(&self, id: u64) {
        let mut state = self.lock();
        if state.children.remove(&id).is_some() {
            state.bucket.total_drip_request = sum_values(&state.children);
        }
    }
}

fn sum_values(children: &BTreeMap<u64, f32>) -> f32 {
    let mut total = 0.0;
    for value in children.values() {
        total += *value;
    }
    total
}

fn modifier(drip: f32, total: f32) -> f32 {
    if drip >= total { 1.0 } else { drip / total }
}

fn scene_drip(scene: &SceneState) -> f32 {
    scene
        .bucket
        .requested_drip()
        .min(scene.bucket.total_drip_request)
}

/// Rates the circuit reads back. Bit patterns are what the C# oracle records.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BucketView {
    pub requested_drip: f32,
    pub drip: f32,
    pub requested_burst: f32,
    pub burst: f32,
    pub total: f32,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TreeView {
    pub scene: BucketView,
    pub client: BucketView,
    pub categories: [BucketView; SLOTS],
}

/// One viewer's client bucket and its category children.
pub(crate) struct ClientThrottle {
    scene: Arc<SceneThrottle>,
    id: u64,
    client: Bucket,
    categories: [Bucket; SLOTS],
    max_rate: f32,
    burst_time: f32,
}

impl Drop for ClientThrottle {
    fn drop(&mut self) {
        self.scene.unregister(self.id);
    }
}

impl ClientThrottle {
    /// `max_rate` is the clamped `client_throttle_max_bps`.
    pub(crate) fn open(scene: Arc<SceneThrottle>, max_rate: f32, now: f64) -> Self {
        let burst_time = BURST_TIME;
        let mut client = Bucket::at(now);
        client.set_requested_drip(max_rate);
        client.set_requested_burst(max_rate * burst_time);
        let id = scene.register(client.drip_rate);

        // Adaptive adjustment is forced off. The client drip stays at its max.
        let max_drip = if max_rate == 0.0 {
            client.total_drip_request
        } else {
            max_rate.max(MIN_FLOW)
        };
        client.drip_rate = max_drip;
        scene.update(id, client.drip_rate);

        let mut categories = [Bucket::at(now); SLOTS];
        for index in 0..SLOTS {
            let rate = DEFAULT_RATES[index];
            categories[index].set_requested_drip(rate);
            categories[index].set_requested_burst(rate * burst_time);
            let total = category_sum(&categories, index + 1);
            client.total_drip_request = total;
            let reported = client.requested_drip().min(total);
            scene.update(id, reported);
        }

        let mut opened = Self {
            scene,
            id,
            client,
            categories,
            max_rate,
            burst_time,
        };
        opened.prime(now);
        opened
    }

    /// The first deposit of a bucket adds nothing. C# takes that empty deposit
    /// when the first packet of a category reaches `RemoveTokens` (it never
    /// calls it for an empty queue), so that packet is refused once and leaves
    /// on the next send pass. Ferroan takes it here, so the first packet of
    /// each category leaves at once. The difference is one send pass.
    fn prime(&mut self, now: f64) {
        for index in 0..SLOTS {
            let (rate, burst) = self.limits(index);
            self.categories[index].drip(now, rate, burst);
        }
    }

    /// Apply one `AgentThrottle` blob. `factor` is the child-agent scale.
    /// A blob shorter than seven floats, or one whose scaled values are not
    /// all numbers, leaves the rates unchanged (D68, TB83, TB84). Infinities
    /// clamp to the cap and the floors.
    pub(crate) fn set_throttles(&mut self, data: &[u8], factor: f32) {
        if data.len() < 28 {
            return;
        }
        let scale = 0.125 * factor;
        let mut rates = [0.0; 7];
        for (index, rate) in rates.iter_mut().enumerate() {
            let mut bytes = [0; 4];
            bytes.copy_from_slice(&data[index * 4..index * 4 + 4]);
            *rate = f32::from_le_bytes(bytes) * scale;
        }
        if rates.iter().any(|rate| rate.is_nan()) {
            return;
        }
        for (index, rate) in rates.iter_mut().enumerate() {
            *rate = rate.clamp(CHANNEL_FLOOR[index], self.max_rate);
        }
        let total: f32 = rates.iter().sum();
        if total > self.max_rate {
            let scale = self.max_rate / total;
            for rate in &mut rates {
                *rate *= scale;
            }
        }
        for index in ASSIGN_ORDER {
            let rate = rates[index];
            self.categories[index].set_requested_drip(rate);
            self.categories[index].set_requested_burst(rate * self.burst_time);
            self.publish_client();
        }
    }

    fn publish_client(&mut self) {
        let total = category_sum(&self.categories, SLOTS);
        self.client.total_drip_request = total;
        let reported = self.client.requested_drip().min(total);
        self.scene.update(self.id, reported);
    }

    pub(crate) fn try_spend(&mut self, category: usize, amount: usize, now: f64) -> bool {
        if category >= SLOTS {
            return false;
        }
        let (rate, burst) = self.limits(category);
        let amount = i32::try_from(amount).unwrap_or(i32::MAX);
        self.categories[category].remove_tokens(amount, now, rate, burst)
    }

    /// How long until `try_spend` can succeed. Zero means it can succeed now.
    pub(crate) fn wait(&self, category: usize, now: f64) -> Duration {
        if category >= SLOTS {
            return Duration::from_secs(3_600);
        }
        let (rate, burst) = self.limits(category);
        if rate == 0.0 {
            return Duration::from_secs(3_600);
        }
        let tokens = tokens_after(self.categories[category], now, rate, burst);
        if !tokens.is_finite() || !rate.is_finite() {
            return Duration::from_secs(3_600);
        }
        if tokens > 0.0 {
            return Duration::ZERO;
        }
        let deficit = f64::from(-tokens);
        duration_from_secs(deficit / f64::from(rate))
    }

    #[cfg(test)]
    pub(crate) fn view(&self) -> TreeView {
        let scene = self.scene.lock();
        let scene_rate = scene_drip(&scene);
        let scene_mod = modifier(scene_rate, scene.bucket.total_drip_request);
        let client_rate = child_drip(
            self.client
                .requested_drip()
                .min(self.client.total_drip_request),
            scene_mod,
        );
        let client_mod = modifier(client_rate, self.client.total_drip_request);
        let mut categories = [zero_view(); SLOTS];
        for index in 0..SLOTS {
            let bucket = self.categories[index];
            let drip = child_drip(
                bucket.requested_drip().min(bucket.total_drip_request),
                client_mod,
            );
            let own = modifier(drip, bucket.total_drip_request);
            categories[index] = BucketView {
                requested_drip: bucket.requested_drip(),
                drip,
                requested_burst: bucket.requested_burst,
                burst: bucket.requested_burst * own,
                total: bucket.total_drip_request,
            };
        }
        TreeView {
            scene: BucketView {
                requested_drip: scene.bucket.requested_drip(),
                drip: scene_rate,
                requested_burst: scene.bucket.requested_burst,
                burst: scene.bucket.requested_burst * scene_mod,
                total: scene.bucket.total_drip_request,
            },
            client: BucketView {
                requested_drip: self.client.requested_drip(),
                drip: client_rate,
                requested_burst: self.client.requested_burst,
                burst: self.client.requested_burst * client_mod,
                total: self.client.total_drip_request,
            },
            categories,
        }
    }

    fn limits(&self, category: usize) -> (f32, f32) {
        let scene = self.scene.lock();
        let scene_rate = scene_drip(&scene);
        let scene_mod = modifier(scene_rate, scene.bucket.total_drip_request);
        let client_rate = child_drip(
            self.client
                .requested_drip()
                .min(self.client.total_drip_request),
            scene_mod,
        );
        let client_mod = modifier(client_rate, self.client.total_drip_request);
        let bucket = self.categories[category];
        let drip = child_drip(
            bucket.requested_drip().min(bucket.total_drip_request),
            client_mod,
        );
        let own = modifier(drip, bucket.total_drip_request);
        (drip, bucket.requested_burst * own)
    }
}

fn category_sum(categories: &[Bucket], count: usize) -> f32 {
    let mut total = 0.0;
    for bucket in categories.iter().take(count) {
        total += bucket.drip_rate;
    }
    total
}

fn child_drip(rate: f32, parent_modifier: f32) -> f32 {
    let mut rate = rate * parent_modifier;
    if rate < MIN_DRIP {
        rate = MIN_DRIP;
    }
    rate
}

#[cfg(test)]
fn zero_view() -> BucketView {
    BucketView {
        requested_drip: 0.0,
        drip: 0.0,
        requested_burst: 0.0,
        burst: 0.0,
        total: 0.0,
    }
}

fn tokens_after(bucket: Bucket, now: f64, drip_rate: f32, burst: f32) -> f32 {
    if drip_rate == 0.0 {
        return bucket.tokens;
    }
    let delta = now - bucket.last_drip;
    if delta <= 0.0 {
        return bucket.tokens;
    }
    let mut tokens = bucket.tokens + (delta as f32) * drip_rate;
    if tokens > burst {
        tokens = burst;
    }
    tokens
}

fn duration_from_secs(secs: f64) -> Duration {
    // A zero deficit still needs a later timestamp: a drip at the same
    // instant adds nothing, and a zero wait would spin the serve loop.
    let secs = if !secs.is_finite() || secs < 1.0e-4 {
        1.0e-4
    } else if secs > 3_600.0 {
        3_600.0
    } else {
        secs + 1.0e-4
    };
    Duration::from_secs_f64(secs)
}

/// Monotonic throttle clock. Tests add `extra` without sleeping.
pub(crate) struct ThrottleClock {
    origin: Instant,
    extra: Duration,
}

impl ThrottleClock {
    pub(crate) fn start() -> Self {
        Self {
            origin: Instant::now(),
            extra: Duration::ZERO,
        }
    }

    pub(crate) fn now_secs(&self) -> f64 {
        self.origin
            .elapsed()
            .saturating_add(self.extra)
            .as_secs_f64()
    }

    #[cfg(test)]
    pub(crate) fn advance(&mut self, by: Duration) {
        self.extra = self.extra.saturating_add(by);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (ClientThrottle, f64) {
        let now = 10.0;
        let client = ClientThrottle::open(SceneThrottle::stock(), CLIENT_MAX_BPS as f32, now);
        (client, now)
    }

    #[test]
    fn the_first_deposit_adds_nothing_and_the_next_can_overdraw() {
        let mut bucket = Bucket::at(10.0);
        bucket.set_requested_drip(18_500.0);
        bucket.set_requested_burst(185.0);
        assert!(!bucket.remove_tokens(40, 10.0, 18_500.0, 185.0));
        assert_eq!(bucket.tokens, 0.0);
        assert!(bucket.remove_tokens(1_000, 10.2, 18_500.0, 185.0));
        assert!(bucket.tokens < 0.0);
        assert!(!bucket.remove_tokens(1, 10.2, 18_500.0, 185.0));
    }

    #[test]
    fn requested_burst_caps_at_the_hard_maximum() {
        let mut bucket = Bucket::at(0.0);
        bucket.set_requested_burst(SCENE_RATE * BURST_TIME);
        assert_eq!(bucket.requested_burst, MAX_BURST);
        bucket.set_requested_drip(-5.0);
        assert_eq!(bucket.drip_rate, 0.0);
    }

    #[test]
    fn a_child_drip_does_not_fall_below_the_minimum() {
        let (mut client, _) = tree();
        let mut blob = [0; 28];
        for index in 0..7 {
            blob[index * 4..index * 4 + 4].copy_from_slice(&1.0f32.to_le_bytes());
        }
        client.set_throttles(&blob, 1.0);
        let view = client.view();
        assert_eq!(view.categories[2].requested_drip, 50.0);
        assert_eq!(view.categories[2].drip, MIN_DRIP);
        assert!(view.categories[TASK].requested_drip >= 1_000.0);
    }

    #[test]
    fn a_short_blob_keeps_the_defaults() {
        let (mut client, _) = tree();
        let before = client.view();
        client.set_throttles(&[0; 8], 1.0);
        assert_eq!(client.view(), before);
    }

    #[test]
    fn a_nan_in_the_blob_leaves_every_rate_unchanged() {
        let scene = SceneThrottle::stock();
        let mut client = ClientThrottle::open(Arc::clone(&scene), CLIENT_MAX_BPS as f32, 1.0);
        let neighbour = ClientThrottle::open(Arc::clone(&scene), CLIENT_MAX_BPS as f32, 1.0);
        let before = (client.view(), neighbour.view());
        for slot in 0..7 {
            let mut blob = ferroan_bot::throttle::initial_throttle_bytes();
            blob[slot * 4..slot * 4 + 4].copy_from_slice(&f32::NAN.to_le_bytes());
            client.set_throttles(&blob, 1.0);
            assert_eq!((client.view(), neighbour.view()), before, "slot {slot}");
        }
        assert_eq!(client.wait(TASK, 2.0), Duration::ZERO);
    }

    #[test]
    fn infinities_clamp_to_the_cap_and_the_floor() {
        let (mut client, _) = tree();
        let mut blob = [0; 28];
        for index in 0..7 {
            let value = if index % 2 == 0 {
                f32::INFINITY
            } else {
                f32::NEG_INFINITY
            };
            blob[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        client.set_throttles(&blob, 1.0);
        let view = client.view();
        for (index, category) in view.categories.iter().take(7).enumerate() {
            assert!(category.requested_drip.is_finite(), "category {index}");
            assert!(category.drip.is_finite(), "category {index}");
        }
    }

    #[test]
    fn a_non_finite_bucket_waits_an_hour() {
        let (mut client, now) = tree();
        client.categories[TASK].tokens = f32::NAN;
        assert_eq!(client.wait(TASK, now + 1.0), Duration::from_secs(3_600));
        let (mut client, now) = tree();
        client.categories[LAND].set_requested_drip(f32::NAN);
        client.publish_client();
        assert_eq!(client.wait(RESEND, now + 1.0), Duration::from_secs(3_600));
    }

    #[test]
    fn a_zero_or_negative_scene_cap_is_unlimited() {
        for scene_max_bps in [0, -5] {
            let scene = SceneThrottle::configured(CircuitThrottle {
                client_max_bps: CLIENT_MAX_BPS,
                scene_max_bps,
            });
            let mut clients = Vec::new();
            let blob = ferroan_bot::throttle::initial_throttle_bytes();
            for _ in 0..20 {
                let mut client = ClientThrottle::open(Arc::clone(&scene), scene.client_max(), 1.0);
                client.set_throttles(&blob, 1.0);
                clients.push(client);
            }
            let view = clients[0].view();
            assert_eq!(view.scene.drip, view.scene.total, "{scene_max_bps}");
            assert_eq!(
                view.categories[TASK].drip, view.categories[TASK].requested_drip,
                "{scene_max_bps}"
            );
        }
    }

    #[test]
    fn the_client_cap_is_clamped_like_throttle_rates() {
        for (asked, kept) in [(100, 6_250), (640_000, 640_000), (5_000_000, 1_000_000)] {
            let scene = SceneThrottle::configured(CircuitThrottle {
                client_max_bps: asked,
                scene_max_bps: SCENE_MAX_BPS,
            });
            assert_eq!(scene.client_max(), kept as f32, "{asked}");
        }
    }

    #[test]
    fn ten_clients_share_one_scene_ceiling() {
        let scene = SceneThrottle::stock();
        let mut clients = Vec::new();
        let kbps: [f32; 7] = [600.0, 400.0, 80.0, 80.0, 2_050.0, 2_050.0, 740.0];
        let blob: Vec<u8> = kbps
            .into_iter()
            .flat_map(|rate| (rate * 1_024.0).to_le_bytes())
            .collect();
        for _ in 0..10 {
            let mut client = ClientThrottle::open(Arc::clone(&scene), CLIENT_MAX_BPS as f32, 1.0);
            client.set_throttles(&blob, 1.0);
            clients.push(client);
        }
        let view = clients[0].view();
        assert!(view.scene.drip <= SCENE_RATE);
        assert!(view.scene.total > SCENE_RATE);
        assert!(view.categories[TASK].drip < view.categories[TASK].requested_drip);
    }

    #[test]
    fn rates_match_the_csharp_oracle() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/throttle/rates.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let document: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            document["revision"].as_str(),
            Some("c401bceecd1684832ae8ba86a31b5ee01a4ded7b")
        );
        let cases = document["cases"].as_array().unwrap();
        let bot = ferroan_bot::throttle::initial_throttle_bytes();
        let mut unchanged = 0;
        for case in cases {
            let name = case["name"].as_str().unwrap();
            if name == "bot_6000" || name == "half" || name == "ten_clients" {
                assert_eq!(decode_hex(case["blob"].as_str().unwrap()), bot, "{name}");
            }
            let actual = replay(case, true);
            if KEPT_RATES.contains(&name) {
                // D68: Ferroan ignores this blob, and C# changes the rates.
                assert_eq!(actual, replay(case, false), "{name}");
                let task = &case["categories"][TASK];
                assert_ne!(
                    task["drip"].as_u64().unwrap() as u32,
                    actual.categories[TASK].drip.to_bits(),
                    "{name} changed nothing in C#"
                );
                unchanged += 1;
                continue;
            }
            assert_snap(&actual.scene, &case["scene"], &format!("{name} scene"));
            assert_snap(&actual.client, &case["client"], &format!("{name} client"));
            let categories = case["categories"].as_array().unwrap();
            assert_eq!(categories.len(), SLOTS, "{name}");
            for (index, expected) in categories.iter().enumerate() {
                assert_snap(
                    &actual.categories[index],
                    expected,
                    &format!("{name} category {index}"),
                );
            }
        }
        assert_eq!(unchanged, KEPT_RATES.len());
    }

    /// Oracle cases where Ferroan keeps the rates (D68, TB83, TB84).
    const KEPT_RATES: [&str; 3] = ["nan_task", "short_0", "short_8"];

    fn replay(case: &serde_json::Value, apply: bool) -> TreeView {
        let scene = SceneThrottle::stock();
        let count = case["clients"].as_u64().unwrap() as usize;
        let mut clients = Vec::new();
        for _ in 0..count {
            let mut client = ClientThrottle::open(Arc::clone(&scene), CLIENT_MAX_BPS as f32, 1.0);
            if apply && case["apply"].as_bool().unwrap() {
                let blob = decode_hex(case["blob"].as_str().unwrap());
                let factor = case["factor"].as_f64().unwrap() as f32;
                client.set_throttles(&blob, factor);
            }
            clients.push(client);
        }
        clients[0].view()
    }

    fn assert_snap(actual: &BucketView, expected: &serde_json::Value, label: &str) {
        let bits = |key: &str| expected[key].as_u64().unwrap() as u32;
        assert_eq!(
            actual.requested_drip.to_bits(),
            bits("requested_drip"),
            "{label} requested"
        );
        assert_eq!(actual.drip.to_bits(), bits("drip"), "{label} drip");
        assert_eq!(
            actual.requested_burst.to_bits(),
            bits("requested_burst"),
            "{label} requested burst"
        );
        assert_eq!(actual.burst.to_bits(), bits("burst"), "{label} burst");
        assert_eq!(actual.total.to_bits(), bits("total"), "{label} total");
    }

    fn decode_hex(text: &str) -> Vec<u8> {
        let bytes = text.as_bytes();
        assert_eq!(bytes.len() % 2, 0, "{text}");
        (0..bytes.len() / 2)
            .map(|index| {
                let nibble = |byte: u8| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    _ => panic!("bad hex in {text}"),
                };
                (nibble(bytes[index * 2]) << 4) | nibble(bytes[index * 2 + 1])
            })
            .collect()
    }
}
