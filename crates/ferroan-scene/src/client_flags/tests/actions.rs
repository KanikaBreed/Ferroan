// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::object_permissions::{ActionPermissions, ObjectAction::*, PresenceState};
use crate::parcels::Parcel;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Probe {
    root_grant: bool,
    part_grant: bool,
    service_member: Option<bool>,
    nil_parcel_group: bool,
    rez_elsewhere: bool,
    force_deny: bool,
    no_handler: bool,
    missing_object: bool,
    missing_presence: bool,
    pub(super) deleted_object: bool,
    deleted_presence: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Decisions {
    can_rez: bool,
    can_edit: bool,
    can_move: bool,
    can_delete: bool,
    can_take: bool,
    can_link: bool,
    can_take_copy: bool,
    can_duplicate: bool,
    can_edit_by_id: bool,
    can_delete_by_id: bool,
    can_delink: bool,
    can_select_object: bool,
}

#[test]
fn twelve_object_decisions_match_pin8_case_by_case() {
    #[derive(Deserialize)]
    struct Parcels {
        parcels: Vec<Parcel>,
    }
    let mut land = serde_json::from_str::<Parcels>(include_str!(
        "../../../../../fixtures/regionstore/parcels.json"
    ))
    .unwrap()
    .parcels
    .remove(0);
    replay_cases(|case, target, mut viewer, administrator| {
        let input = &case.input;
        let expected = &case.expected.decisions;
        let probe = &input.actions;
        if probe.no_handler {
            viewer.mode = PermissionMode::NoModule;
        }
        let presence = if probe.missing_presence {
            PresenceState::Missing
        } else if probe.deleted_presence {
            PresenceState::Deleted
        } else {
            PresenceState::Active
        };
        let permissions = ActionPermissions {
            agent: viewer.agent,
            is_god: viewer.is_god,
            mode: viewer.mode,
            membership: viewer.membership,
            presence,
            administrator,
            bypass_value: !probe.force_deny,
            service_group_powers: &|group| {
                (group == input.membership_group(target.root.group_id)
                    && probe.service_member.unwrap_or(input.member))
                .then_some(input.powers)
            },
            friend_grant: &|grantee, owner| {
                owner == target.root.owner_id
                    && ((input.friend_can_modify && grantee == viewer.agent)
                        || (probe.root_grant && grantee == target.root.uuid)
                        || (probe.part_grant && grantee == target.part.uuid))
            },
        };
        land.group = if probe.nil_parcel_group {
            Uuid::nil()
        } else {
            Uuid::from_u128(3)
        };
        land.owner = if input.owns_land {
            viewer.agent
        } else if input.group_land {
            Uuid::from_u128(3)
        } else {
            Uuid::from_u128(5)
        };
        land.group_owned = input.group_land;
        land.flags = input.land_flags;
        let parcel = input.has_land.then_some(&land);
        let object = (!probe.missing_object).then_some(target);
        let name = &input.name;
        assert_eq!(
            permissions.can_rez(parcel.filter(|_| !probe.rez_elsewhere)),
            expected.can_rez,
            "{name}: canRez"
        );
        assert_eq!(
            permissions.can_select(object),
            expected.can_select_object,
            "{name}: canSelectObject"
        );
        for (label, action, observed) in [
            ("canEdit", Edit, expected.can_edit),
            ("canMove", Move, expected.can_move),
            ("canDelete", Delete, expected.can_delete),
            ("canTake", Take, expected.can_take),
            ("canLink", Link, expected.can_link),
            ("canTakeCopy", TakeCopy, expected.can_take_copy),
            ("canDuplicate", Duplicate(parcel), expected.can_duplicate),
            ("canEditById", EditById, expected.can_edit_by_id),
            ("canDeleteById", DeleteById, expected.can_delete_by_id),
            ("canDelink", Delink, expected.can_delink),
        ] {
            assert_eq!(
                permissions.allows(action, object),
                observed,
                "{name}: {label}"
            );
        }
    });
}
