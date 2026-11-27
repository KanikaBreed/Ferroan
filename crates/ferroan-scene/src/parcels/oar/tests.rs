// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashMap, HashSet};
use std::future::Future;

use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::bitmap::{Grid, Placement, remap};
use super::land_xml::{base64, boolean, byte, deserialize, int32, omv_uuid, sbyte, single, uint32};
use super::*;
use crate::oar_owners::{OwnerLookups, OwnerOptions, OwnerResolver};
use crate::parcels::{LandAccess, limit_hits, vector_text};

const FIXTURE: &str = include_str!("../../../../../fixtures/oar-parcels/parcels.json");

/// Pin8 loses its place in these files (TB203). Ferroan reads them as the
/// `local` file they were made from (D139); `bad-environment` loads with the
/// unreadable environment cleared, which is `local`'s.
const KNOWN_DIFFERENCES: [&str; 4] = [
    "bad-last-value",
    "bad-environment",
    "bad-last-in-entry",
    "nested-value",
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    revision: String,
    estate_owner: Uuid,
    users: Vec<Uuid>,
    groups: Vec<Uuid>,
    existing: Vec<ExistingRow>,
    files: HashMap<String, String>,
    file_ids: HashMap<String, Uuid>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExistingRow {
    global_id: Uuid,
    local_id: i32,
    name: String,
    owner_id: Uuid,
    bitmap: String,
}

#[derive(Deserialize)]
struct Case {
    input: Input,
    expected: Expected,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    name: String,
    files: Vec<String>,
    region_size: u32,
    displacement: Option<[f32; 3]>,
    rotation: f32,
    bounding_origin: Option<[f32; 3]>,
    bounding_size: Option<[f32; 3]>,
    merge: bool,
    merge_parcels: bool,
    groups_module: bool,
    zero_estate_owner: bool,
    existing: Option<Vec<Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expected {
    error: Option<String>,
    clear: Vec<Clear>,
    parcels: Option<Vec<Value>>,
    kept: usize,
    calls: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Clear {
    setup_default_parcel: bool,
}

/// The services the oracle registered, logging each call as it does.
#[derive(Default)]
struct Grid2 {
    users: HashSet<Uuid>,
    groups: Option<HashSet<Uuid>>,
    calls: Vec<String>,
}

impl OwnerLookups for Grid2 {
    fn user_exists(&mut self, id: Uuid) -> impl Future<Output = bool> + Send {
        self.calls.push(format!("GetUserAccount {id}"));
        let exists = self.users.contains(&id);
        async move { exists }
    }

    fn group_exists(&mut self, id: Uuid) -> impl Future<Output = bool> + Send {
        let exists = self.groups.as_ref().map(|groups| {
            self.calls.push(format!("GetGroupRecord {id}"));
            groups.contains(&id)
        });
        async move { exists.unwrap_or(false) }
    }

    async fn alias_user(&mut self, _alias: Uuid) -> Option<Uuid> {
        panic!("LoadParcels never asks for an alias")
    }

    fn add_creator_user(&mut self, _id: Uuid, _creator_data: &str) {
        panic!("LoadParcels never adds a creator");
    }
}

fn resolver(users: &[Uuid], groups: Option<&[Uuid]>) -> OwnerResolver<Grid2> {
    let grid = Grid2 {
        users: users.iter().copied().collect(),
        groups: groups.map(|groups| groups.iter().copied().collect()),
        calls: Vec::new(),
    };
    OwnerResolver::new(grid, OwnerOptions::new(None, Uuid::nil()))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The oracle's record of one `LandData`. An id that is neither a file's nor
/// an existing parcel's is `random`.
fn record(parcel: &OarParcel, known: &HashSet<Uuid>) -> Value {
    let p = &parcel.data;
    let id = if known.contains(&p.id) {
        p.id.to_string()
    } else {
        "random".to_owned()
    };
    let mut map = serde_json::Map::new();
    let mut put = |key: &str, value: Value| {
        map.insert(key.to_owned(), value);
    };
    put("globalId", json!(id));
    put("localId", json!(p.local_id));
    put("name", json!(p.name));
    put("description", json!(p.description));
    put("ownerId", json!(p.owner.to_string()));
    put("groupId", json!(p.group.to_string()));
    put("isGroupOwned", json!(p.group_owned));
    put("area", json!(p.area));
    put("auctionId", json!(p.auction));
    put("authBuyerId", json!(p.auth_buyer.to_string()));
    put("category", json!(p.category));
    put("claimDate", json!(p.claim_date));
    put("claimPrice", json!(p.claim_price));
    put("salePrice", json!(p.sale_price));
    put("status", json!(p.status));
    put("flags", json!(p.flags));
    put("landingType", json!(p.landing_type));
    put("mediaAutoScale", json!(p.media_auto_scale));
    put("mediaId", json!(p.media_id.to_string()));
    put("mediaUrl", json!(p.media_url));
    put("musicUrl", json!(p.music_url));
    put("passHours", json!(p.pass_hours));
    put("passPrice", json!(p.pass_price));
    put("snapshotId", json!(p.snapshot.to_string()));
    put("dwell", json!(p.dwell));
    put("mediaType", json!(p.media_type));
    put("mediaDescription", json!(p.media_description));
    put("mediaWidth", json!(p.media_size[0]));
    put("mediaHeight", json!(p.media_size[1]));
    put("mediaLoop", json!(p.media_loop));
    put("obscureMedia", json!(p.obscure_media));
    put("obscureMusic", json!(p.obscure_music));
    put("seeAvs", json!(p.see_avs));
    put("anyAvSounds", json!(p.any_av_sounds));
    put("groupAvSounds", json!(p.group_av_sounds));
    put("userLocation", json!(p.location));
    put("userLookAt", json!(p.look_at));
    put("otherCleanTime", json!(p.other_clean_time));
    put("simwideArea", json!(p.simwide_area));
    put("environment", json!(p.environment.is_some()));
    put("environmentVersion", json!(p.environment_version));
    put("bitmap", json!(b64(&parcel.bitmap)));
    put(
        "access",
        json!(
            p.access
                .iter()
                .map(|entry| json!({
                    "agentId": entry.agent.to_string(),
                    "flags": entry.flags,
                    "expires": entry.expires,
                }))
                .collect::<Vec<_>>()
        ),
    );
    Value::Object(map)
}

/// Integers compare exactly, other numbers as the single-precision floats
/// both sides hold.
fn same(got: &Value, want: &Value) -> bool {
    match (got, want) {
        (Value::Number(a), Value::Number(b)) => match (a.as_i64(), b.as_i64()) {
            (Some(a), Some(b)) => a == b,
            _ => match (a.as_u64(), b.as_u64()) {
                (Some(a), Some(b)) => a == b,
                _ => a.as_f64().map(|v| v as f32) == b.as_f64().map(|v| v as f32),
            },
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same(value, other)))
        }
        _ => got == want,
    }
}

fn assert_parcel(label: &str, got: &Value, want: &Value) {
    if same(got, want) {
        return;
    }
    let mut lines = Vec::new();
    if let (Value::Object(a), Value::Object(b)) = (got, want) {
        for (key, value) in b {
            if !a.get(key).is_some_and(|other| same(other, value)) {
                lines.push(format!("  {key}: got {:?}, Pin8 {value}", a.get(key)));
            }
        }
    }
    panic!("{label} differs from Pin8:\n{}", lines.join("\n"));
}

fn existing_parcels(fixture: &Fixture) -> Vec<OarParcel> {
    fixture
        .existing
        .iter()
        .map(|row| {
            let mut data = Parcel::estate_land(row.owner_id, 0);
            data.id = row.global_id;
            data.local_id = row.local_id;
            data.name = row.name.clone();
            OarParcel {
                data,
                bitmap: base64(&row.bitmap).unwrap(),
            }
        })
        .collect()
}

fn options_for(input: &Input, estate_owner: Uuid) -> ParcelOptions {
    let estate = if input.zero_estate_owner {
        Uuid::nil()
    } else {
        estate_owner
    };
    let mut options = ParcelOptions::new([input.region_size; 2], estate);
    options.merge = input.merge;
    options.merge_parcels = input.merge_parcels;
    options.displacement = input.displacement.unwrap_or([0.0; 3]);
    options.rotation = input.rotation;
    if let Some(origin) = input.bounding_origin {
        options = options.bounding_origin(origin);
    }
    if let Some(size) = input.bounding_size {
        options = options.bounding_size(size);
    }
    options
}

async fn run_case(
    fixture: &Fixture,
    input: &Input,
) -> (Result<ParcelLoad, OarParcelError>, Vec<String>) {
    let groups = input.groups_module.then_some(fixture.groups.as_slice());
    let mut resolver = resolver(&fixture.users, groups);
    let files: Vec<&[u8]> = input
        .files
        .iter()
        .map(|name| fixture.files[name].as_bytes())
        .collect();
    let existing = if input.existing.is_some() {
        existing_parcels(fixture)
    } else {
        Vec::new()
    };
    let options = options_for(input, fixture.estate_owner);
    let result = load_parcels(&mut resolver, &files, existing, &options).await;
    (result, resolver.into_lookups().calls)
}

#[tokio::test]
async fn every_parcel_load_matches_pin8() {
    let fixture: Fixture = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture.revision, ferroan_diag::TRANQUILLITY_PIN);
    let mut known: HashSet<Uuid> = fixture.file_ids.values().copied().collect();
    known.extend(fixture.existing.iter().map(|row| row.global_id));
    let default_local = fixture
        .cases
        .iter()
        .find(|case| case.input.name == "default")
        .and_then(|case| case.expected.parcels.as_ref())
        .map(|parcels| parcels[0].clone())
        .unwrap();
    assert_eq!(default_local["name"], "West & field");

    for case in &fixture.cases {
        let name = case.input.name.as_str();
        let (result, calls) = run_case(&fixture, &case.input).await;
        if KNOWN_DIFFERENCES.contains(&name) {
            let Ok(ParcelLoad::Replace {
                loaded,
                kept,
                setup_default,
            }) = result
            else {
                panic!("{name}: Ferroan reads the file in place, got {result:?}");
            };
            assert!(kept.is_empty() && !setup_default, "{name}");
            assert_eq!(loaded.len(), 1, "{name}");
            assert_parcel(name, &record(&loaded[0], &known), &default_local);
            // Pin8 failed or read another parcel.
            let pin8 = case.expected.parcels.as_ref().map(|p| p[0].clone());
            assert!(
                case.expected.error.is_some() || pin8.is_some_and(|p| !same(&p, &default_local)),
                "{name}: Pin8 no longer differs; TB203 may be fixed"
            );
            continue;
        }
        assert_eq!(calls, case.expected.calls, "{name}: service calls");
        if let Some(error) = &case.expected.error {
            assert!(
                result.is_err(),
                "{name}: Pin8 threw {error}, got {result:?}"
            );
            continue;
        }
        let result = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        let Some(want) = &case.expected.parcels else {
            assert!(case.expected.clear.is_empty(), "{name}");
            assert_eq!(result, ParcelLoad::Unchanged, "{name}");
            continue;
        };
        let ParcelLoad::Replace {
            loaded,
            kept,
            setup_default,
        } = result
        else {
            panic!("{name}: Pin8 replaced the parcels");
        };
        assert_eq!(case.expected.clear.len(), 1, "{name}");
        assert_eq!(
            setup_default, case.expected.clear[0].setup_default_parcel,
            "{name}"
        );
        assert_eq!(kept.len(), case.expected.kept, "{name}: kept parcels");
        let got: Vec<Value> = loaded
            .iter()
            .chain(&kept)
            .map(|parcel| record(parcel, &known))
            .collect();
        assert_eq!(got.len(), want.len(), "{name}: parcel count");
        for (index, (got, want)) in got.iter().zip(want).enumerate() {
            assert_parcel(&format!("{name} parcel {index}"), got, want);
        }
    }
}

// The branches, each on its own.

fn id(tail: u32) -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0000 | u128::from(tail))
}

const ESTATE: u32 = 0xe1;
const LOCAL: u32 = 0xa1;
const STRANGER: u32 = 0xb1;
const GROUP: u32 = 0xf1;
const NO_GROUP: u32 = 0xf2;

fn whole_region_bits() -> String {
    b64(&[0xff; 512])
}

fn parcel_xml(body: &str) -> String {
    format!(
        "<LandData><Bitmap>{}</Bitmap>{body}</LandData>",
        whole_region_bits()
    )
}

async fn load_one(body: &str) -> (OarParcel, Vec<String>) {
    let mut resolver = resolver(&[id(ESTATE), id(LOCAL)], Some(&[id(GROUP)]));
    let options = ParcelOptions::new([256, 256], id(ESTATE));
    let load = load_parcels(&mut resolver, &[parcel_xml(body)], Vec::new(), &options)
        .await
        .unwrap();
    let ParcelLoad::Replace { mut loaded, .. } = load else {
        panic!("one parcel loads");
    };
    (loaded.remove(0), resolver.into_lookups().calls)
}

#[tokio::test]
async fn group_owner_and_access_resolve_as_pin8_does() {
    let (parcel, calls) = load_one(&format!(
        "<OwnerID>{}</OwnerID><GroupID>{}</GroupID>",
        id(LOCAL),
        id(NO_GROUP)
    ))
    .await;
    assert_eq!(
        parcel.data.group,
        Uuid::nil(),
        "an unknown group is cleared"
    );
    assert_eq!(parcel.data.owner, id(LOCAL), "an account keeps its parcel");
    assert_eq!(
        calls,
        [
            format!("GetGroupRecord {}", id(NO_GROUP)),
            format!("GetUserAccount {}", id(LOCAL)),
        ]
    );

    let (parcel, _) = load_one(&format!("<OwnerID>{}</OwnerID>", id(STRANGER))).await;
    assert_eq!(
        parcel.data.owner,
        id(ESTATE),
        "a stranger's parcel goes to the estate owner"
    );

    let (parcel, calls) = load_one(&format!(
        "<OwnerID>{}</OwnerID><GroupID>{}</GroupID><IsGroupOwned>true</IsGroupOwned>",
        id(STRANGER),
        id(GROUP)
    ))
    .await;
    assert!(parcel.data.group_owned);
    assert_eq!(
        parcel.data.owner,
        id(GROUP),
        "a group-owned parcel is its group's"
    );
    assert_eq!(
        calls,
        [format!("GetGroupRecord {}", id(GROUP))],
        "the owner is not asked"
    );

    let (parcel, _) = load_one(&format!(
        "<OwnerID>{}</OwnerID><GroupID>{}</GroupID><IsGroupOwned>True</IsGroupOwned>",
        id(LOCAL),
        id(NO_GROUP)
    ))
    .await;
    assert!(
        !parcel.data.group_owned,
        "with no group it is no longer group-owned"
    );
    assert_eq!(parcel.data.owner, id(ESTATE));

    let entry = |agent: Uuid, flags: u32| {
        format!(
            "<ParcelAccessEntry><AgentID>{agent}</AgentID><Time>99</Time><AccessList>{flags}</AccessList></ParcelAccessEntry>"
        )
    };
    let (parcel, calls) = load_one(&format!(
        "<ParcelAccessList>{}{}{}</ParcelAccessList>",
        entry(id(LOCAL), 1),
        entry(id(STRANGER), 2),
        entry(Uuid::nil(), 1),
    ))
    .await;
    assert_eq!(
        parcel.data.access,
        [LandAccess {
            agent: id(LOCAL),
            flags: 1,
            expires: 0,
        }],
        "only accounts stay, and Time is dropped"
    );
    // The zero id is never looked up, and the zero owner went to the estate owner.
    assert_eq!(parcel.data.owner, id(ESTATE));
    assert!(
        !calls
            .iter()
            .any(|call| call.contains(&Uuid::nil().to_string()))
    );
}

#[tokio::test]
async fn one_cache_serves_the_load() {
    let mut resolver = resolver(&[id(LOCAL)], Some(&[id(GROUP)]));
    let options = ParcelOptions::new([256, 256], id(ESTATE));
    let body = format!(
        "<OwnerID>{}</OwnerID><GroupID>{}</GroupID>",
        id(LOCAL),
        id(GROUP)
    );
    let files = [parcel_xml(&body), parcel_xml(&body)];
    load_parcels(&mut resolver, &files, Vec::new(), &options)
        .await
        .unwrap();
    assert_eq!(resolver.lookups().calls.len(), 2, "each id is asked once");
}

#[tokio::test]
async fn nothing_to_load_leaves_the_parcels() {
    let options = ParcelOptions::new([256, 256], id(ESTATE));
    let no_files: [&[u8]; 0] = [];
    let mut lookups = resolver(&[], None);
    assert_eq!(
        load_parcels(&mut lookups, &no_files, Vec::new(), &options)
            .await
            .unwrap(),
        ParcelLoad::Unchanged
    );
    // --merge without --merge-parcels never collects the files.
    let mut merge = options;
    merge.merge = true;
    let files = [parcel_xml("")];
    assert_eq!(
        load_parcels(&mut lookups, &files, Vec::new(), &merge)
            .await
            .unwrap(),
        ParcelLoad::Unchanged
    );
    // A file with no land clears the region and sets up the default parcel.
    let empty = [format!(
        "<LandData><Bitmap>{}</Bitmap></LandData>",
        b64(&[0; 512])
    )];
    assert_eq!(
        load_parcels(&mut lookups, &empty, Vec::new(), &options)
            .await
            .unwrap(),
        ParcelLoad::Replace {
            loaded: Vec::new(),
            kept: Vec::new(),
            setup_default: true,
        }
    );
    // A missing Bitmap is LandData's 512 zero bytes.
    let missing = [String::from("<LandData><Name>x</Name></LandData>")];
    let load = load_parcels(&mut lookups, &missing, Vec::new(), &options)
        .await
        .unwrap();
    assert!(matches!(
        load,
        ParcelLoad::Replace {
            setup_default: true,
            ..
        }
    ));
    assert!(
        lookups.lookups().calls.is_empty(),
        "skipped parcels ask nothing"
    );
}

#[tokio::test]
async fn merge_trims_and_drops_the_region_parcels() {
    let west = |x_max: usize| {
        let mut grid = vec![0u8; 512];
        for y in 0..64 {
            for x in 0..x_max {
                let n = y * 64 + x;
                grid[n / 8] |= 1 << (n % 8);
            }
        }
        grid
    };
    let old = |name: &str, bitmap: Vec<u8>| OarParcel {
        data: {
            let mut data = Parcel::estate_land(id(LOCAL), 0);
            data.name = name.into();
            data
        },
        bitmap,
    };
    let mut options = ParcelOptions::new([256, 256], id(ESTATE));
    options.merge = true;
    options.merge_parcels = true;
    let file = [format!(
        "<LandData><GlobalID>{}</GlobalID><Bitmap>{}</Bitmap></LandData>",
        id(0x1a),
        b64(&west(32))
    )];
    let existing = vec![old("covered", west(8)), old("wider", west(48))];
    let mut lookups = resolver(&[], None);
    let ParcelLoad::Replace {
        loaded,
        kept,
        setup_default,
    } = load_parcels(&mut lookups, &file, existing, &options)
        .await
        .unwrap()
    else {
        panic!("a merge replaces");
    };
    assert!(!setup_default);
    assert_ne!(loaded[0].data.id, id(0x1a), "a merged parcel gets a new id");
    assert_eq!(kept.len(), 1, "a covered parcel is dropped");
    assert_eq!(kept[0].data.name, "wider");
    let wider = Grid::from_bytes(&kept[0].bitmap);
    let expected: Vec<u8> = west(48)
        .iter()
        .zip(west(32))
        .map(|(all, taken)| all & !taken)
        .collect();
    assert_eq!(wider, Grid::from_bytes(&expected));

    // A region parcel of another size fails the load, as Pin8's throw does.
    let mut lookups = resolver(&[], None);
    let result = load_parcels(
        &mut lookups,
        &file,
        vec![old("small", vec![0xff; 8])],
        &options,
    )
    .await;
    assert!(
        matches!(result, Err(OarParcelError::BitmapSize { .. })),
        "{result:?}"
    );
}

#[test]
fn bounding_size_is_clipped_to_the_region() {
    let options = ParcelOptions::new([256, 512], Uuid::nil());
    assert_eq!(options.bounding().1, [256.0, 512.0, 50_100.0]);
    let clipped = options.bounding_size([1000.0, -1.0, 7.0]);
    assert_eq!(clipped.bounding().1, [256.0, 512.0, 7.0]);
    let kept = options
        .bounding_origin([8.0, 8.0, 0.0])
        .bounding_size([64.0, 32.0, 1.0]);
    assert_eq!(kept.bounding(), ([8.0, 8.0, 0.0], [64.0, 32.0, 1.0]));
}

fn grid_with(side: usize, cells: &[(usize, usize)]) -> Grid {
    let mut bytes = vec![0u8; side * side / 8];
    for &(x, y) in cells {
        let n = y * side + x;
        bytes[n / 8] |= 1 << (n % 8);
    }
    Grid::from_bytes(&bytes)
}

fn placement(rotation: f32) -> Placement {
    Placement {
        displacement: [0.0; 2],
        rotation,
        bounding_origin: [0.0; 2],
        bounding_size: [32.0; 2],
        region_size: [32.0; 2],
    }
}

#[test]
fn orthogonal_turns_move_a_cell_exactly() {
    // An 8 x 8 source: offset 2, working grid 12, centre 6.
    let source = grid_with(8, &[(1, 0)]);
    let at = |rotation| {
        let (grid, empty) = remap(&source, &placement(rotation));
        assert!(!empty);
        grid
    };
    assert_eq!(at(0.0), grid_with(8, &[(1, 0)]));
    assert_eq!(at(90.0), grid_with(8, &[(7, 1)]));
    assert_eq!(at(180.0), grid_with(8, &[(6, 7)]));
    assert_eq!(at(270.0), grid_with(8, &[(0, 6)]));
    assert_eq!(at(-90.0), at(270.0));
    assert_eq!(at(-360.0), at(0.0));
}

#[test]
fn a_remap_clips_displaces_and_can_empty() {
    let source = grid_with(8, &[(0, 0), (7, 7)]);
    let mut place = placement(0.0);
    place.displacement = [4.0, 0.0];
    let (grid, _) = remap(&source, &place);
    assert_eq!(
        grid,
        grid_with(8, &[(1, 0)]),
        "the far cell moves off the region"
    );
    place.bounding_origin = [8.0, 8.0];
    place.bounding_size = [4.0, 4.0];
    place.displacement = [0.0; 2];
    let (grid, empty) = remap(&source, &place);
    assert!(empty && grid.is_empty(), "nothing inside the bounding box");
    // A larger destination keeps the source at its origin.
    place = placement(0.0);
    place.region_size = [64.0; 2];
    let (grid, _) = remap(&source, &place);
    assert_eq!(grid, grid_with(16, &[(0, 0), (7, 7)]));
}

#[test]
fn bitmap_bytes_follow_the_override_conversion() {
    // 3 bytes are 24 bits: a 4 x 4 square, and the third byte is past it.
    assert!(Grid::from_bytes(&[0, 0, 0xff]).is_empty());
    assert_eq!(Grid::from_bytes(&[1]).size(), [2, 2]);
    assert_eq!(Grid::from_bytes(&[]).size(), [0, 0]);
    let grid = grid_with(8, &[(3, 5)]);
    assert_eq!(grid.to_bytes().unwrap(), {
        let mut bytes = vec![0; 8];
        bytes[5] = 1 << 3;
        bytes
    });
    // A 2 x 2 grid with a set cell has no whole byte: Pin8 throws.
    assert!(Grid::from_bytes(&[1]).to_bytes().is_err());
    assert_eq!(Grid::from_bytes(&[0]).to_bytes().unwrap(), Vec::<u8>::new());
}

fn read(xml: &str) -> Result<OarParcel, OarParcelError> {
    deserialize(xml, &mut 0)
}

#[test]
fn land_data_reads_each_element_in_place() {
    assert!(matches!(read("<Land/>"), Err(OarParcelError::NotLandData(name)) if name == "Land"));
    assert!(matches!(
        read("<LandData/>"),
        Err(OarParcelError::EmptyLandData)
    ));
    assert!(matches!(
        read("<LandData><Name>x</LandData>"),
        Err(OarParcelError::Xml(_))
    ));

    let fresh = read("<LandData></LandData>").unwrap();
    assert_eq!(fresh.data.name, "Your Parcel");
    assert_eq!(fresh.bitmap, vec![0; 512]);
    assert_ne!(
        fresh.data.id,
        Uuid::nil(),
        "LandData starts with a random id"
    );
    assert_eq!(fresh.data.environment_version, -1);

    // A bad value keeps the earlier one, and so does a value with child elements.
    let parcel = read(
        "<LandData><Area>5</Area><Area>x</Area><Name>a</Name><Name><b/></Name>\
         <Dwell>3</Dwell><Unknown><Area>9</Area></Unknown><LocalID>2</LocalID></LandData>",
    )
    .unwrap();
    assert_eq!((parcel.data.area, parcel.data.name.as_str()), (5, "a"));
    assert_eq!(parcel.data.local_id, 2, "the read goes on after a failure");
    assert_eq!(parcel.data.dwell, 0.0, "Dwell is not read");

    // The tenth failure stops the read; anything after it fails the file.
    let bad = "<Area>x</Area>".repeat(10);
    assert!(matches!(
        read(&format!("<LandData>{bad}<Name>n</Name></LandData>")),
        Err(OarParcelError::TooManyErrors)
    ));
    let at_end = read(&format!("<LandData><Name>n</Name>{bad} \n</LandData>")).unwrap();
    assert_eq!(at_end.data.name, "n");
    let nine = "<Area>x</Area>".repeat(9);
    assert_eq!(
        read(&format!("<LandData>{nine}<Name>n</Name></LandData>"))
            .unwrap()
            .data
            .name,
        "n"
    );

    // The flags strings the serializer writes.
    let parcel = read(
        "<LandData><SeeAVs>0</SeeAVs><AnyAVSnds>true</AnyAVSnds><GrpAVSnds>1</GrpAVSnds></LandData>",
    )
    .unwrap();
    assert!(!parcel.data.see_avs && !parcel.data.any_av_sounds && parcel.data.group_av_sounds);
}

#[test]
fn access_entries_read_until_another_element() {
    let parcel = read(
        "<LandData><ParcelAccessList>\n <ParcelAccessEntry><AccessList>x</AccessList><AgentID>\
         00000000-0000-4000-8000-0000000000a1</AgentID><Time>5</Time><Unknown/></ParcelAccessEntry>\
         <ParcelAccessEntry/><ParcelAccessEntry></ParcelAccessEntry><Other/>\
         <ParcelAccessEntry><AccessList>4</AccessList></ParcelAccessEntry></ParcelAccessList>\
         <ParcelAccessList><ParcelAccessEntry><AccessList>-0</AccessList></ParcelAccessEntry>\
         </ParcelAccessList></LandData>",
    )
    .unwrap();
    let agents: Vec<_> = parcel
        .data
        .access
        .iter()
        .map(|e| (e.agent, e.flags, e.expires))
        .collect();
    assert_eq!(
        agents,
        [(id(LOCAL), 0, 0), (Uuid::nil(), 0, 0), (Uuid::nil(), 0, 0)],
        "an empty element adds nothing, a start and end tag adds a zero entry, \
         entries after another element are not read, and lists add up"
    );
    assert_eq!(
        read("<LandData><ParcelAccessList/></LandData>")
            .unwrap()
            .data
            .access,
        []
    );
}

#[test]
fn an_unreadable_environment_is_cleared_and_keeps_its_version() {
    let good = "&lt;llsd&gt;&lt;map&gt;&lt;key&gt;env_version&lt;/key&gt;&lt;integer&gt;42&lt;/integer&gt;&lt;/map&gt;&lt;/llsd&gt;";
    let parcel = read(&format!(
        "<LandData><Environment>{good}</Environment></LandData>"
    ))
    .unwrap();
    assert_eq!(parcel.data.environment_version, 42);
    assert!(
        parcel
            .data
            .environment
            .as_deref()
            .is_some_and(|text| text.starts_with("<llsd>"))
    );
    let parcel = read(&format!(
        "<LandData><Environment>{good}</Environment><Environment>no</Environment></LandData>"
    ))
    .unwrap();
    assert_eq!(
        (
            parcel.data.environment.is_none(),
            parcel.data.environment_version
        ),
        (true, 42)
    );
    assert!(
        read("<LandData><Environment/></LandData>")
            .unwrap()
            .data
            .environment
            .is_none()
    );
}

#[test]
fn values_convert_as_dotnet_does() {
    assert_eq!(int32(" \t42 \0\0"), Some(42));
    assert_eq!(int32("+7"), Some(7));
    assert_eq!(int32("4\0 "), None);
    assert_eq!(int32(""), None);
    assert_eq!(int32("2147483648"), None);
    assert_eq!(uint32("-0"), Some(0));
    assert_eq!(uint32("-00"), Some(0));
    assert_eq!(uint32("-1"), None);
    assert_eq!(uint32("-"), None);
    assert_eq!(byte("255"), Some(255));
    assert_eq!(byte("256"), None);
    assert_eq!(sbyte("-128"), Some(-128));
    assert_eq!(sbyte("128"), None);
    assert_eq!(boolean(" TRUE\0"), Some(true));
    assert_eq!(boolean("False"), Some(false));
    assert_eq!(boolean("1"), None);
    assert_eq!(single("1,234.5"), Some(1234.5));
    assert_eq!(single(" -2.5e1 "), Some(-25.0));
    assert_eq!(single("∞"), Some(f32::INFINITY));
    assert_eq!(single("Infinity"), None);
    assert_eq!(single(",5"), None);
    assert_eq!(single(""), None);
    assert_eq!(base64("AQ\n ID"), Some(vec![1, 2, 3]));
    assert_eq!(base64("AQ=="), Some(vec![1]));
    assert_eq!(base64("AQI="), Some(vec![1, 2]));
    assert_eq!(base64(""), Some(vec![]));
    assert_eq!(base64("AQ"), None);
    assert_eq!(base64("A==="), None);
    assert_eq!(base64("AQ==AQ=="), None);
    assert_eq!(base64("AQ=A"), None);
    assert_eq!(base64("AQ\u{c}I="), None, "a form feed is not ignored");
}

#[test]
fn uuids_parse_as_openmetaverse_does() {
    let text = "00000000-0000-4000-8000-0000000000a1";
    assert_eq!(omv_uuid(text), Some(id(LOCAL)));
    assert_eq!(omv_uuid(&format!("  {text}")), Some(id(LOCAL)));
    assert_eq!(omv_uuid(&format!("{text}trailing")), Some(id(LOCAL)));
    assert_eq!(omv_uuid(&text.to_uppercase()), Some(id(LOCAL)));
    assert_eq!(omv_uuid(&text.replace('-', "")), Some(id(LOCAL)));
    assert_eq!(
        omv_uuid(&format!("{{{text}}}")),
        None,
        "braces are not read"
    );
    assert_eq!(
        omv_uuid(&format!("\t{text}")),
        None,
        "only spaces are skipped"
    );
    assert_eq!(omv_uuid(&text[..35]), None);
    assert_eq!(omv_uuid(&text.replace('a', "g")), None);
    assert_eq!(omv_uuid(""), None);
}

#[test]
fn vector3_values_follow_the_acceptance_check() {
    use vector_text::Components::Triple;
    for text in [
        "<1, 2, 3>",
        "<1,2,3",
        "1,2,3",
        "x<1,2,3>",
        "<1,2,3 4>",
        "<1,2>",
        "<,1,2,3>",
        "<1,2,>",
        "<a,b,c>",
        "<∞,-∞,NaN>",
        "<Infinity,1,2>",
        "1,2,3\0",
    ] {
        assert_eq!(
            vector_text::vector3(text).is_some(),
            vector_text::accepts(text, Triple),
            "{text}"
        );
    }
    assert_eq!(vector_text::vector3("<1, 2.5, -3>"), Some([1.0, 2.5, -3.0]));
}

#[tokio::test]
async fn every_bound_refuses_the_load() {
    // Only this test reaches these four ids, so the counters need no lock.
    let options = ParcelOptions::new([256, 256], id(ESTATE));
    let mut lookups = resolver(&[], None);

    let before = limit_hits("oar.parcels", "refuse");
    let files = vec![Vec::<u8>::new(); MAX_OAR_PARCELS + 1];
    let result = load_parcels(&mut lookups, &files, Vec::new(), &options).await;
    assert!(matches!(result, Err(OarParcelError::Limit("oar.parcels"))));
    assert_eq!(limit_hits("oar.parcels", "refuse"), before + 1);

    let before = limit_hits("oar.parcel-xml-bytes", "refuse");
    let files = vec![vec![b' '; MAX_PARCEL_XML_BYTES / 2 + 1]; 2];
    let result = load_parcels(&mut lookups, &files, Vec::new(), &options).await;
    assert!(matches!(
        result,
        Err(OarParcelError::Limit("oar.parcel-xml-bytes"))
    ));
    assert_eq!(limit_hits("oar.parcel-xml-bytes", "refuse"), before + 1);

    let before = limit_hits("oar.parcel-access", "refuse");
    let half = "<ParcelAccessEntry></ParcelAccessEntry>".repeat(MAX_ACCESS_ENTRIES / 2);
    let file = parcel_xml(&format!("<ParcelAccessList>{half}</ParcelAccessList>"));
    let result = load_parcels(&mut lookups, &[&file, &file], Vec::new(), &options).await;
    assert!(result.is_ok(), "the bound counts the whole load");
    let over = parcel_xml(&format!(
        "<ParcelAccessList>{half}<ParcelAccessEntry></ParcelAccessEntry></ParcelAccessList>"
    ));
    let result = load_parcels(&mut lookups, &[&file, &over], Vec::new(), &options).await;
    assert!(matches!(
        result,
        Err(OarParcelError::Limit("oar.parcel-access"))
    ));
    assert_eq!(limit_hits("oar.parcel-access", "refuse"), before + 1);

    let before = limit_hits("oar.parcel-bitmap-bytes", "refuse");
    let file = format!(
        "<LandData><Bitmap>{}</Bitmap></LandData>",
        b64(&vec![0xff; MAX_BITMAP_BYTES])
    );
    assert!(read(&file).is_ok());
    let file = format!(
        "<LandData><Bitmap>{}</Bitmap></LandData>",
        b64(&vec![0xff; MAX_BITMAP_BYTES + 1])
    );
    let result = load_parcels(&mut lookups, &[file], Vec::new(), &options).await;
    assert!(matches!(
        result,
        Err(OarParcelError::Limit("oar.parcel-bitmap-bytes"))
    ));
    assert_eq!(limit_hits("oar.parcel-bitmap-bytes", "refuse"), before + 1);
}

#[tokio::test]
async fn loaded_bitmaps_are_bounded_per_load() {
    // Only this test reaches `oar.parcel-bitmaps`.
    let options = ParcelOptions::new([256, 256], id(ESTATE));
    let mut lookups = resolver(&[], None);
    let land = parcel_xml("");
    let no_land = format!("<LandData><Bitmap>{}</Bitmap></LandData>", b64(&[0; 512]));
    // A 256 m region's bitmap is 512 bytes. A skipped parcel holds none.
    let two = [&land, &no_land, &land];
    let load = load_within(&mut lookups, &two, Vec::new(), &options, 1024)
        .await
        .unwrap();
    assert!(matches!(load, ParcelLoad::Replace { ref loaded, .. } if loaded.len() == 2));
    let before = limit_hits("oar.parcel-bitmaps", "refuse");
    let three = [&land, &land, &land];
    let result = load_within(&mut lookups, &three, Vec::new(), &options, 1024).await;
    assert!(matches!(
        result,
        Err(OarParcelError::Limit("oar.parcel-bitmaps"))
    ));
    assert_eq!(limit_hits("oar.parcel-bitmaps", "refuse"), before + 1);
    // The budget is the region's bitmap size times the parcels kept, so a
    // 4096 m region holds 2,048 under the default.
    assert_eq!(MAX_LOADED_BITMAP_BYTES / (1024 * 1024 / 8), 2048);
}
