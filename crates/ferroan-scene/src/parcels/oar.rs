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
//   Source/OpenSim.Region.CoreModules/World/Archiver/ArchiveReadRequest.cs
//   (LoadParcels, the parcel branch of DearchiveRegion and the constructor's
//   merge, displacement, rotation and bounding options)
//   OpenSim-Tranquillity `dev-build` @ c401bceecd1684832ae8ba86a31b5ee01a4ded7b
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The parcels of an OAR load, resolved in memory (`LoadParcels`).
//!
//! [`load_parcels`] reads each `landdata/` file ([`land_xml`]), in archive
//! order. A parcel with no claimed land is skipped. Its bitmap is turned,
//! clipped to the bounding rectangle and moved by the displacement into a
//! bitmap the size of this region ([`bitmap`]); a parcel left with no land
//! is skipped. Then, at `ArchiveReadRequest.cs` 973–1003:
//!
//! - A group with no group record becomes the zero group.
//! - A group-owned parcel is owned by its group. With the zero group it is
//!   the estate owner's and no longer group-owned.
//! - Any other parcel whose owner is not an account becomes the estate
//!   owner's. The estate owner is used even when `default-user` is set.
//! - An access entry is kept only when its agent is an account.
//!
//! The lookups are [`OwnerResolver`]'s: the same caches serve the whole load,
//! parcels first and then objects, as in Pin8.
//!
//! `--merge` reads parcel files only with `--merge-parcels`. A merged parcel
//! gets a new global id, and the land it takes is cleared from the region's
//! parcels, which are kept unless nothing is left of them.
//!
//! The result is what Pin8 hands `LandChannel.Clear` and
//! `TriggerIncomingLandDataFromStorage`. Installing and storing it is
//! P2.0e4.6b. No parcel file leaves the region's parcels as they are.
//!
//! A file that does not read fails the whole load and installs nothing, as
//! Pin8's exception does. So does a file past `oar.parcels`,
//! `oar.parcel-xml-bytes`, `oar.parcel-access` or `oar.parcel-bitmap-bytes`,
//! and a parcel whose region-sized bitmap would take the loaded parcels past
//! `oar.parcel-bitmaps`.

mod bitmap;
mod land_xml;
#[cfg(test)]
mod tests;

use ferroan_diag::{Kind, OAR_PARCEL_BITMAPS, OAR_PARCEL_XML_BYTES, OAR_PARCELS, refuse};
use uuid::Uuid;

use super::Parcel;
use crate::oar_owners::{OwnerLookups, OwnerResolver};
use bitmap::{Grid, Placement};

/// Parcel files one load reads. Pin8 has no bound. The same as `region.parcels`.
pub const MAX_OAR_PARCELS: usize = 65_536;
/// Bytes in all the parcel files of one load. Pin8 has no bound.
pub const MAX_PARCEL_XML_BYTES: usize = 64 * 1024 * 1024;
/// Access entries in all the parcel files of one load, before the filter.
pub const MAX_ACCESS_ENTRIES: usize = 65_536;
/// Bytes in one parcel's decoded bitmap: a 4096 m square region.
pub const MAX_BITMAP_BYTES: usize = 1024 * 1024 / 8;
/// Bytes of region-sized bitmaps the loaded parcels of one load hold. Every
/// loaded parcel holds one, whatever its file's size: 65,536 parcels in a
/// 256 m region, 2,048 in a 4096 m one. Pin8 has no bound.
pub const MAX_LOADED_BITMAP_BYTES: usize = 256 * 1024 * 1024;

/// `Constants.MinSimulationHeight` and `MaxSimulationHeight`.
const MIN_SIMULATION_HEIGHT: f32 = -100.0;
const MAX_SIMULATION_HEIGHT: f32 = 50_000.0;

/// One parcel as `LandData` holds it: the fields and its bitmap bytes.
#[derive(Debug, PartialEq)]
pub struct OarParcel {
    pub data: Parcel,
    pub bitmap: Vec<u8>,
}

/// What a load does to the region's parcels.
#[derive(Debug, PartialEq)]
pub enum ParcelLoad {
    /// No parcel file reached `LoadParcels`. The region's parcels stay as
    /// they are, and the default parcel is not set up.
    Unchanged,
    /// `LandChannel.Clear(setup_default)`, then
    /// `TriggerIncomingLandDataFromStorage` with `loaded` and then `kept`.
    Replace {
        /// The archive's parcels, resolved, in archive order.
        loaded: Vec<OarParcel>,
        /// With `--merge --merge-parcels`, the region's parcels that still
        /// hold land, with the merged land cleared, in their order.
        kept: Vec<OarParcel>,
        /// No archive parcel was loaded, so `Clear` sets up the default parcel.
        setup_default: bool,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum OarParcelError {
    #[error("parcel XML: {0}")]
    Xml(String),
    #[error("parcel XML root is {0}, not LandData")]
    NotLandData(String),
    #[error("parcel XML has an empty LandData element")]
    EmptyLandData,
    #[error("parcel XML has ten unreadable values before its end")]
    TooManyErrors,
    #[error("parcel bitmap has a partial last byte")]
    BitmapTail,
    #[error("land bitmaps are not the same size: base {base:?}, new {new:?}")]
    BitmapSize { base: [usize; 2], new: [usize; 2] },
    #[error("OAR parcels exceed {0}")]
    Limit(&'static str),
}

/// The constructor's options that `LoadParcels` reads.
#[derive(Clone, Copy, Debug)]
pub struct ParcelOptions {
    /// `RegionInfo.RegionSizeX` and `RegionSizeY` of the region loading.
    pub region_size: [u32; 2],
    /// `RegionInfo.EstateSettings.EstateOwner`.
    pub estate_owner: Uuid,
    /// `--merge`.
    pub merge: bool,
    /// `--merge-parcels` (also `force-parcels` and `forceparcels`).
    pub merge_parcels: bool,
    /// `displacement`, in metres.
    pub displacement: [f32; 3],
    /// `rotation`, in degrees. The console clamps it to ±359.
    pub rotation: f32,
    bounding_origin: [f32; 3],
    bounding_size: [f32; 3],
}

impl ParcelOptions {
    /// No displacement, no rotation and the bounding box of the region.
    pub fn new(region_size: [u32; 2], estate_owner: Uuid) -> Self {
        Self {
            region_size,
            estate_owner,
            merge: false,
            merge_parcels: false,
            displacement: [0.0; 3],
            rotation: 0.0,
            bounding_origin: [0.0, 0.0, MIN_SIMULATION_HEIGHT],
            bounding_size: Self::region_box(region_size),
        }
    }

    fn region_box(region_size: [u32; 2]) -> [f32; 3] {
        [
            region_size[0] as f32,
            region_size[1] as f32,
            MAX_SIMULATION_HEIGHT - MIN_SIMULATION_HEIGHT,
        ]
    }

    /// `bounding-origin`, as given.
    pub fn bounding_origin(mut self, origin: [f32; 3]) -> Self {
        self.bounding_origin = origin;
        self
    }

    /// `bounding-size`. A side that is not positive, or is larger than the
    /// region, becomes the region's side.
    pub fn bounding_size(mut self, size: [f32; 3]) -> Self {
        let region = Self::region_box(self.region_size);
        let mut size = size;
        let mut clip = false;
        for axis in 0..2 {
            // NaN is neither, and stays.
            if size[axis] <= 0.0 || size[axis] > region[axis] {
                size[axis] = region[axis];
                clip = true;
            }
        }
        if clip {
            tracing::info!(
                "[ARCHIVER]: The bounding cube specified is larger than the destination region! Clipping to <{}, {}, {}>.",
                size[0],
                size[1],
                size[2]
            );
        }
        self.bounding_size = size;
        self
    }

    pub fn bounding(&self) -> ([f32; 3], [f32; 3]) {
        (self.bounding_origin, self.bounding_size)
    }

    fn placement(&self) -> Placement {
        Placement {
            displacement: [self.displacement[0], self.displacement[1]],
            rotation: self.rotation,
            bounding_origin: [self.bounding_origin[0], self.bounding_origin[1]],
            bounding_size: [self.bounding_size[0], self.bounding_size[1]],
            region_size: [self.region_size[0] as f32, self.region_size[1] as f32],
        }
    }
}

fn refuse_limit<E>(id: &'static str, max: usize, observed: usize, error: E) -> Result<(), E> {
    if observed <= max {
        return Ok(());
    }
    let configured = u64::try_from(max).unwrap_or(u64::MAX);
    let observed = u64::try_from(observed).unwrap_or(u64::MAX);
    refuse(id, Kind::Hard, configured, observed, "oar-load");
    Err(error)
}

/// `LoadParcels` over the archive's parcel files, in archive order.
/// `existing` is `LandChannel.AllParcels()`, read only for a merge.
pub async fn load_parcels<L: OwnerLookups, F: AsRef<[u8]>>(
    resolver: &mut OwnerResolver<L>,
    files: &[F],
    existing: Vec<OarParcel>,
    options: &ParcelOptions,
) -> Result<ParcelLoad, OarParcelError> {
    load_within(resolver, files, existing, options, MAX_LOADED_BITMAP_BYTES).await
}

/// [`load_parcels`] with the `oar.parcel-bitmaps` budget supplied.
async fn load_within<L: OwnerLookups, F: AsRef<[u8]>>(
    resolver: &mut OwnerResolver<L>,
    files: &[F],
    existing: Vec<OarParcel>,
    options: &ParcelOptions,
    bitmap_budget: usize,
) -> Result<ParcelLoad, OarParcelError> {
    // DearchiveRegion collects parcel files only when `!m_merge || m_mergeParcels`.
    if options.merge && !options.merge_parcels || files.is_empty() {
        tracing::info!("[ARCHIVER]: No parcels to load, or skiping load");
        return Ok(ParcelLoad::Unchanged);
    }
    refuse_limit(
        OAR_PARCELS,
        MAX_OAR_PARCELS,
        files.len(),
        OarParcelError::Limit(OAR_PARCELS),
    )?;
    let bytes = files
        .iter()
        .fold(0usize, |sum, file| sum.saturating_add(file.as_ref().len()));
    refuse_limit(
        OAR_PARCEL_XML_BYTES,
        MAX_PARCEL_XML_BYTES,
        bytes,
        OarParcelError::Limit(OAR_PARCEL_XML_BYTES),
    )?;

    tracing::info!("[ARCHIVER]: Loading {} parcels.  Please wait.", files.len());
    let merge = options.merge && options.merge_parcels;
    let place = options.placement();
    let mut existing: Vec<Option<OarParcel>> = existing.into_iter().map(Some).collect();
    let mut loaded = Vec::new();
    let mut access_used = 0;
    // ConvertLandBitmapToBytes on the region-sized result.
    let region_bitmap =
        (options.region_size[0] as usize / 4) * (options.region_size[1] as usize / 4) / 8;
    let mut bitmaps_held = 0usize;

    for file in files {
        // Encoding.UTF8.GetString.
        let text = String::from_utf8_lossy(file.as_ref());
        let mut parcel = land_xml::deserialize(&text, &mut access_used)?;
        let source = Grid::from_bytes(&parcel.bitmap);
        if source.is_empty() {
            tracing::info!(
                "[ARCHIVER]: Skipping source parcel {} with GlobalID: {} LocalID: {} that has no claimed land.",
                parcel.data.name,
                parcel.data.id,
                parcel.data.local_id
            );
            continue;
        }
        bitmaps_held = bitmaps_held.saturating_add(region_bitmap);
        refuse_limit(
            OAR_PARCEL_BITMAPS,
            bitmap_budget,
            bitmaps_held,
            OarParcelError::Limit(OAR_PARCEL_BITMAPS),
        )?;
        let (land, empty) = bitmap::remap(&source, &place);
        if empty {
            tracing::warn!(
                "[ARCHIVER]: Not adding destination parcel {} with GlobalID: {} LocalID: {} because, after applying rotation, bounding and displacement, it has no claimed land.",
                parcel.data.name,
                parcel.data.id,
                parcel.data.local_id
            );
            continue;
        }
        parcel.bitmap = land.to_bytes().map_err(|_| OarParcelError::BitmapTail)?;

        if merge {
            // A new id, in case the same archive is merged twice.
            parcel.data.id = Uuid::new_v4();
            for slot in &mut existing {
                let Some(old) = slot else {
                    continue;
                };
                let mut grid = Grid::from_bytes(&old.bitmap);
                let gone = grid
                    .remove(&land)
                    .map_err(|size| OarParcelError::BitmapSize {
                        base: size.base,
                        new: size.taken,
                    })?;
                if gone {
                    *slot = None;
                } else {
                    old.bitmap = grid.to_bytes().map_err(|_| OarParcelError::BitmapTail)?;
                }
            }
        }

        resolve(resolver, &mut parcel.data, options.estate_owner).await;
        loaded.push(parcel);
    }

    let kept: Vec<OarParcel> = if merge {
        existing.into_iter().flatten().collect()
    } else {
        Vec::new()
    };
    if merge {
        tracing::info!("[ARCHIVER]: Keeping {} old parcels.", kept.len());
    }
    tracing::info!(
        "[ARCHIVER]: Added {} total parcels.",
        loaded.len() + kept.len()
    );
    Ok(ParcelLoad::Replace {
        setup_default: loaded.is_empty(),
        loaded,
        kept,
    })
}

/// `ArchiveReadRequest.cs` 971–1003: the group, the owner and the access list.
async fn resolve<L: OwnerLookups>(
    resolver: &mut OwnerResolver<L>,
    parcel: &mut Parcel,
    estate_owner: Uuid,
) {
    if !resolver.resolve_group(parcel.group).await {
        parcel.group = Uuid::nil();
    }
    if parcel.group_owned {
        if parcel.group.is_nil() {
            parcel.owner = estate_owner;
            parcel.group_owned = false;
        } else {
            // In group-owned parcels, OwnerID=GroupID.
            parcel.owner = parcel.group;
        }
    } else if !resolver.resolve_user(parcel.owner).await {
        parcel.owner = estate_owner;
    }
    let mut kept = Vec::with_capacity(parcel.access.len());
    for entry in std::mem::take(&mut parcel.access) {
        if resolver.resolve_user(entry.agent).await {
            kept.push(entry);
        }
    }
    parcel.access = kept;
}
