// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared types for Ferroan.
//!
//! UUID, vector, quaternion, colour, texture-entry, extra-parameter, prim-shape
//! and permission-mask bytes match OpenMetaverse and Tranquillity.

mod extra;
mod permissions;
mod shape;
mod texture;
mod wire;

pub use extra::{ExtraParams, FlexibleData, LightData, LightImage, MAX_EXTRA_BYTES, SculptData};
pub use glam::{DVec3, Quat, Vec2, Vec3, Vec4};
pub use permissions::{
    PERM_ALL, PERM_ALL_AND_EXPORT, PERM_ALL_AND_EXPORT_NO_MOD, PERM_ALL_EFFECTIVE, PERM_COPY,
    PERM_DAMAGE, PERM_EXPORT, PERM_FOLDED_COPY, PERM_FOLDED_EXPORT, PERM_FOLDED_MASK,
    PERM_FOLDED_MODIFY, PERM_FOLDED_TRANSFER, PERM_FOLDING_SHIFT, PERM_MODIFY, PERM_MOVE,
    PERM_NONE, PERM_SLAM, PERM_TRANSFER, PERM_UNFOLDED_MASK, PERMISSIONS_LEN, Permissions,
    apply_folded_permissions, apply_no_mod_folded_permissions, fix_and_fold_permissions,
};
pub use shape::{
    HOLE_TRIANGLE, PATH_CIRCLE, PATH_LINE, PROFILE_SQUARE, SHAPE_LEN, ShapeParams, pack_begin_cut,
    pack_end_cut, pack_path_revolutions, pack_path_scale, pack_path_shear, pack_path_taper,
    pack_path_twist, pack_profile_hollow, profile_curve, profile_hole, set_profile_curve,
    set_profile_hole, swap_nibbles, unpack_begin_cut, unpack_end_cut, unpack_path_revolutions,
    unpack_path_scale, unpack_path_shear, unpack_path_taper, unpack_path_twist,
    unpack_profile_hollow,
};
pub use texture::{
    MAX_FACES, MAX_TEXTURE_BYTES, TextureEntry, TextureFaceSet, TextureFaceView, WHITE_TEXTURE,
};
pub use wire::{
    Color4, WireError, float_to_u16_bounds, float_to_u16_range, float_to_unit_u16,
    float_zero_one_to_byte, read_color4, read_color4_floats, read_dvec3, read_quaternion_full,
    read_quaternion_packed, read_quaternion_shorts, read_u16_value, read_uuid, read_vec2,
    read_vec3, read_vec3_clamped, read_vec4, u16_to_float, write_color4, write_color4_floats,
    write_dvec3, write_quaternion_full, write_quaternion_get_bytes, write_quaternion_shorts,
    write_quaternion_to_bytes, write_u16, write_uuid, write_vec2, write_vec3, write_vec3_clamped,
    write_vec4,
};

/// The OpenSim Tranquillity `develop` commit that Ferroan's ports and
/// compatibility checks currently follow. Ported files record their own
/// revision; this is the starting point.
pub const REFERENCE_TRANQUILLITY_REVISION: &str = "713857f10b8e91a639a967d4354a4682b080793c";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_revision_is_a_full_commit_hash() {
        assert_eq!(REFERENCE_TRANQUILLITY_REVISION.len(), 40);
        assert!(
            REFERENCE_TRANQUILLITY_REVISION
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}
