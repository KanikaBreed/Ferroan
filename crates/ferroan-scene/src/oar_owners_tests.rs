// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashMap, HashSet};
use std::future::Future;

use serde::Deserialize;
use uuid::Uuid;

use super::{OwnerLookups, OwnerOptions, OwnerResolver};
use crate::objects::ObjectGroup;

const FIXTURE: &str = include_str!("../../../fixtures/oar-owners/objects.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    revision: String,
    estate_owner: Uuid,
    default_user: Uuid,
    users: Vec<Uuid>,
    groups: Vec<Uuid>,
    aliases: Vec<AliasRow>,
    objects: Vec<String>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct AliasRow {
    alias: Uuid,
    user: Uuid,
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
    default_user_option: bool,
    no_default_user: bool,
    lookup_aliases: bool,
    groups_module: bool,
    alias_service: bool,
    user_management: bool,
    zero_estate_owner: bool,
}

#[derive(Deserialize)]
struct Expected {
    objects: Vec<ExpectedObject>,
    calls: Vec<String>,
}

#[derive(Deserialize)]
struct ExpectedObject {
    parts: Vec<Expect>,
}

/// One part, or one item when `uuid` is absent and `item_id` is set.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expect {
    #[serde(default)]
    uuid: Option<Uuid>,
    #[serde(default)]
    item_id: Option<Uuid>,
    creator_id: Uuid,
    creator_data: String,
    owner_id: Uuid,
    last_owner_id: Uuid,
    group_id: Uuid,
    #[serde(default)]
    items: Vec<Expect>,
}

/// The services the oracle registered, answering from the fixture's rows and
/// logging each call the way the oracle does.
#[derive(Default)]
struct Grid {
    users: HashSet<Uuid>,
    groups: Option<HashSet<Uuid>>,
    aliases: Option<HashMap<Uuid, Uuid>>,
    user_management: bool,
    calls: Vec<String>,
}

impl OwnerLookups for Grid {
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

    fn alias_user(&mut self, alias: Uuid) -> impl Future<Output = Option<Uuid>> + Send {
        let user = self.aliases.as_ref().and_then(|aliases| {
            self.calls.push(format!("GetUserForAlias {alias}"));
            aliases.get(&alias).copied()
        });
        async move { user }
    }

    fn add_creator_user(&mut self, id: Uuid, creator_data: &str) {
        if self.user_management {
            self.calls
                .push(format!("AddCreatorUser {id} {creator_data}"));
        }
    }
}

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).unwrap()
}

fn read(xml: &str) -> ObjectGroup {
    crate::xml::from_xml2(xml).expect("fixture XML reads")
}

fn check(case: &str, at: &str, expect: &Expect, creator: (Uuid, String), ids: [Uuid; 3]) {
    let fields = [
        ("creatorId", expect.creator_id, creator.0),
        ("ownerId", expect.owner_id, ids[0]),
        ("lastOwnerId", expect.last_owner_id, ids[1]),
        ("groupId", expect.group_id, ids[2]),
    ];
    for (field, want, got) in fields {
        assert_eq!(got, want, "case {case} {at} {field}");
    }
    assert_eq!(
        creator.1, expect.creator_data,
        "case {case} {at} creatorData"
    );
}

#[tokio::test]
async fn every_rewrite_and_lookup_matches_pin8() {
    let fixture = fixture();
    assert_eq!(fixture.revision, ferroan_diag::TRANQUILLITY_PIN);
    assert_eq!(fixture.cases.len(), 12);
    for case in &fixture.cases {
        let input = &case.input;
        let name = &input.name;
        let grid = Grid {
            users: fixture.users.iter().copied().collect(),
            groups: input
                .groups_module
                .then(|| fixture.groups.iter().copied().collect()),
            aliases: input.alias_service.then(|| {
                fixture
                    .aliases
                    .iter()
                    .map(|row| (row.alias, row.user))
                    .collect()
            }),
            user_management: input.user_management,
            calls: Vec::new(),
        };
        let estate_owner = if input.zero_estate_owner {
            Uuid::nil()
        } else {
            fixture.estate_owner
        };
        let mut options = OwnerOptions::new(
            input.default_user_option.then_some(fixture.default_user),
            estate_owner,
        );
        options.no_default_user = input.no_default_user;
        options.lookup_aliases = input.lookup_aliases;
        let mut resolver = OwnerResolver::new(grid, options);
        assert_eq!(fixture.objects.len(), case.expected.objects.len());
        for (index, (xml, expected)) in fixture
            .objects
            .iter()
            .zip(&case.expected.objects)
            .enumerate()
        {
            let mut group = read(xml);
            resolver.modify_scene_object(&mut group).await;
            assert_eq!(group.parts.len(), expected.parts.len(), "case {name}");
            for (part, expect) in group.parts.iter().zip(&expected.parts) {
                let at = format!("object {index} part {}", part.uuid);
                assert_eq!(Some(part.uuid), expect.uuid, "case {name} {at}");
                let creator = crate::xml::part_creator(&part.creator_id).0;
                let data = part.xml_creator_data.clone().unwrap_or_default();
                let ids = [part.owner_id, part.last_owner_id, part.group_id];
                check(name, &at, expect, (creator, data), ids);
                assert_eq!(part.items.len(), expect.items.len(), "case {name} {at}");
                for (item, want) in part.items.iter().zip(&expect.items) {
                    let at = format!("{at} item {}", item.item_id);
                    assert_eq!(Some(item.item_id), want.item_id, "case {name} {at}");
                    let creator = crate::xml::item_creator(&item.creator_id).0;
                    let data = item.xml_creator_data.clone().unwrap_or_default();
                    let ids = [item.owner_id, item.last_owner_id, item.group_id];
                    check(name, &at, want, (creator, data), ids);
                }
            }
        }
        assert_eq!(
            resolver.lookups().calls,
            case.expected.calls,
            "case {name} service calls"
        );
    }
}

// The branch tests below use these ids and the fixture's one-part object.
const ESTATE: Uuid = Uuid::from_u128(0xe1);
const DEFAULT: Uuid = Uuid::from_u128(0xd1);
const LOCAL: Uuid = Uuid::from_u128(0xa1);
const LOCAL2: Uuid = Uuid::from_u128(0xa2);
const STRANGER: Uuid = Uuid::from_u128(0xb1);
const ALIAS: Uuid = Uuid::from_u128(0xc1);
const ALIAS_AWAY: Uuid = Uuid::from_u128(0xc2);
const GROUP: Uuid = Uuid::from_u128(0xf1);
const NO_GROUP: Uuid = Uuid::from_u128(0xf2);

fn grid() -> Grid {
    Grid {
        users: HashSet::from([ESTATE, DEFAULT, LOCAL, LOCAL2]),
        groups: Some(HashSet::from([GROUP])),
        aliases: Some(HashMap::from([(ALIAS, LOCAL2), (ALIAS_AWAY, STRANGER)])),
        user_management: true,
        calls: Vec::new(),
    }
}

fn single(creator: Uuid, owner: Uuid, last_owner: Uuid, group: Uuid) -> ObjectGroup {
    let mut object = read(&fixture().objects[1]);
    let part = &mut object.parts[0];
    part.creator_id = creator.to_string();
    part.xml_creator_data = None;
    part.owner_id = owner;
    part.last_owner_id = last_owner;
    part.group_id = group;
    let item = &mut part.items[0];
    item.creator_id = creator.to_string();
    item.xml_creator_data = None;
    item.owner_id = owner;
    item.last_owner_id = last_owner;
    item.group_id = group;
    object
}

fn options(lookup_aliases: bool, no_default_user: bool) -> OwnerOptions {
    let mut options = OwnerOptions::new(None, ESTATE);
    options.lookup_aliases = lookup_aliases;
    options.no_default_user = no_default_user;
    options
}

#[test]
fn the_default_user_option_wins_over_the_estate_owner() {
    assert_eq!(
        OwnerOptions::new(Some(DEFAULT), ESTATE).default_user,
        DEFAULT
    );
    let plain = OwnerOptions::new(None, ESTATE);
    assert_eq!(plain.default_user, ESTATE);
    assert!(!plain.no_default_user && !plain.lookup_aliases);
}

#[tokio::test]
async fn zero_ids_are_never_asked_about_as_accounts_or_groups() {
    let mut resolver = OwnerResolver::new(grid(), options(false, false));
    let nil = Uuid::nil();
    let mut object = single(nil, nil, nil, nil);
    resolver.modify_scene_object(&mut object).await;
    let part = &object.parts[0];
    assert_eq!(
        (part.owner_id, part.last_owner_id, part.group_id),
        (ESTATE, ESTATE, nil)
    );
    assert_eq!(part.creator_id, ESTATE.to_string());
    assert_eq!(
        part.items[0].last_owner_id, nil,
        "an item's last owner stays"
    );
    assert!(
        resolver
            .lookups()
            .calls
            .iter()
            .all(|call| call.starts_with("AddCreatorUser")),
        "{:?}",
        resolver.lookups().calls
    );
}

#[tokio::test]
async fn known_accounts_and_groups_stay_and_each_id_is_asked_once() {
    let mut resolver = OwnerResolver::new(grid(), options(false, false));
    for _ in 0..2 {
        let mut object = single(LOCAL, LOCAL2, GROUP, GROUP);
        resolver.modify_scene_object(&mut object).await;
        let part = &object.parts[0];
        assert_eq!(part.creator_id, LOCAL.to_string());
        assert_eq!(
            (part.owner_id, part.last_owner_id, part.group_id),
            (LOCAL2, GROUP, GROUP)
        );
        let item = &part.items[0];
        assert_eq!((item.owner_id, item.group_id), (LOCAL2, GROUP));
    }
    let calls = &resolver.lookups().calls;
    let asked = |prefix: &str| calls.iter().filter(|call| call.starts_with(prefix)).count();
    // LOCAL, LOCAL2 and GROUP as an account; GROUP as a group.
    assert_eq!(asked("GetUserAccount"), 3);
    assert_eq!(asked("GetGroupRecord"), 1);
    assert_eq!(asked("AddCreatorUser"), 4);
}

#[tokio::test]
async fn a_group_is_an_owner_but_never_a_creator() {
    let mut resolver = OwnerResolver::new(grid(), options(false, false));
    let mut object = single(GROUP, GROUP, GROUP, NO_GROUP);
    resolver.modify_scene_object(&mut object).await;
    let part = &object.parts[0];
    assert_eq!(part.creator_id, ESTATE.to_string());
    assert_eq!((part.owner_id, part.last_owner_id), (GROUP, GROUP));
    assert_eq!(part.group_id, Uuid::nil(), "an unknown group is cleared");
    assert_eq!(part.items[0].creator_id, ESTATE.to_string());
    assert_eq!(part.items[0].group_id, Uuid::nil());
}

#[tokio::test]
async fn creator_data_keeps_the_creator_and_reaches_user_management() {
    let mut resolver = OwnerResolver::new(grid(), options(false, false));
    let mut object = single(STRANGER, LOCAL, LOCAL, GROUP);
    let part = &mut object.parts[0];
    part.xml_creator_data = Some("http://hg.example:8002;Visitor".into());
    part.creator_id = format!("{STRANGER};http://hg.example:8002;Visitor");
    // A store load carries the data in the identification only.
    part.items[0].creator_id = format!("{STRANGER};http://hg.example:8002/;Visitor");
    resolver.modify_scene_object(&mut object).await;
    let part = &object.parts[0];
    assert_eq!(
        part.creator_id,
        format!("{STRANGER};http://hg.example:8002;Visitor")
    );
    assert_eq!(
        part.items[0].creator_id,
        format!("{STRANGER};http://hg.example:8002/;Visitor")
    );
    let calls = &resolver.lookups().calls;
    assert!(!calls.contains(&format!("GetUserAccount {STRANGER}")));
    assert!(calls.contains(&format!(
        "AddCreatorUser {STRANGER} http://hg.example:8002;Visitor"
    )));
    assert!(calls.contains(&format!(
        "AddCreatorUser {STRANGER} http://hg.example:8002/;Visitor"
    )));
}

#[tokio::test]
async fn an_alias_resolves_only_to_an_account_and_only_when_asked() {
    for (lookup, no_default, want_alias, want_away) in [
        (false, false, ESTATE, ESTATE),
        (true, false, LOCAL2, ESTATE),
        (false, true, ALIAS, ALIAS_AWAY),
        (true, true, LOCAL2, ALIAS_AWAY),
    ] {
        let mut resolver = OwnerResolver::new(grid(), options(lookup, no_default));
        let mut object = single(ALIAS, ALIAS, ALIAS_AWAY, GROUP);
        resolver.modify_scene_object(&mut object).await;
        let part = &object.parts[0];
        let at = format!("lookup {lookup} no-default {no_default}");
        assert_eq!(part.creator_id, want_alias.to_string(), "{at}");
        assert_eq!(part.owner_id, want_alias, "{at}");
        assert_eq!(part.last_owner_id, want_away, "{at}");
        assert_eq!(part.items[0].owner_id, want_alias, "{at}");
        let asked = resolver
            .lookups()
            .calls
            .iter()
            .filter(|call| call.starts_with("GetUserForAlias"))
            .count();
        assert_eq!(asked, if lookup { 2 } else { 0 }, "{at}");
    }
}

#[tokio::test]
async fn an_absent_alias_is_remembered() {
    let mut resolver = OwnerResolver::new(grid(), options(true, false));
    let mut object = single(STRANGER, STRANGER, STRANGER, GROUP);
    resolver.modify_scene_object(&mut object).await;
    assert_eq!(object.parts[0].owner_id, ESTATE);
    let calls = &resolver.lookups().calls;
    assert_eq!(
        calls
            .iter()
            .filter(|call| **call == format!("GetUserForAlias {STRANGER}"))
            .count(),
        1
    );
}

#[tokio::test]
async fn missing_services_answer_no_group_and_no_alias() {
    let mut lookups = grid();
    lookups.groups = None;
    lookups.aliases = None;
    lookups.user_management = false;
    let mut resolver = OwnerResolver::new(lookups, options(true, false));
    let mut object = single(ALIAS, GROUP, ALIAS, GROUP);
    resolver.modify_scene_object(&mut object).await;
    let part = &object.parts[0];
    assert_eq!(
        (part.owner_id, part.last_owner_id, part.group_id),
        (ESTATE, ESTATE, Uuid::nil())
    );
    assert!(
        resolver
            .lookups()
            .calls
            .iter()
            .all(|call| call.starts_with("GetUserAccount"))
    );
}

#[tokio::test]
async fn a_full_cache_repeats_the_call_and_keeps_the_answer() {
    let mut resolver = OwnerResolver::with_cache_limit(grid(), options(false, false), 2);
    for _ in 0..2 {
        let mut object = single(LOCAL, STRANGER, LOCAL2, GROUP);
        resolver.modify_scene_object(&mut object).await;
        let part = &object.parts[0];
        assert_eq!(part.creator_id, LOCAL.to_string());
        assert_eq!(
            (part.owner_id, part.last_owner_id, part.group_id),
            (ESTATE, LOCAL2, GROUP)
        );
    }
    // The zero id and LOCAL fill the account cache, so STRANGER and LOCAL2
    // are asked again each time they appear.
    let calls = &resolver.lookups().calls;
    let count = |call: String| calls.iter().filter(|seen| **seen == call).count();
    assert_eq!(count(format!("GetUserAccount {LOCAL}")), 1);
    assert_eq!(count(format!("GetUserAccount {STRANGER}")), 4);
    assert_eq!(count(format!("GetUserAccount {LOCAL2}")), 2);
    assert_eq!(resolver.users.len(), 2);
    assert_eq!(resolver.groups.len(), 2);
}
