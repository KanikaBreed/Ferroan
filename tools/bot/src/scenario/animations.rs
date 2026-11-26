// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `animations`: the second bot watches the first walk, stop, then start and
//! stop an animation of its own (`AgentAnimation`). Every `AvatarAnimation`
//! it gets for the walker is kept. Pin8 sends the walker's `STAND` sequence 1
//! when the watcher arrives (`SendOtherAgentsAvatarFullToMe`), `WALK` on the
//! first forward update, `STAND` one number on after the release
//! (`ScenePresenceAnimator`), then the started animation after the default
//! with the next number, and the default alone once it stops
//! (`HandleAgentAnimation`). The numbers start from a clock, so only their
//! steps are checked here; the harness compares the bytes.

use std::time::Duration;

use ferroan_protocol::msgs::{AgentAnimation, agent_animation};
use uuid::Uuid;

use super::observer::ObserverSession;
use super::{Harness, enter_observed_pair, finish_observed_walk, observed_forward};

/// `STAND` and `WALK` (libopenmetaverse `Animations`).
pub const STAND: Uuid = Uuid::from_u128(0x2408fe9e_df1d_1d7d_f4ff_1384fa7b350f);
pub const WALK: Uuid = Uuid::from_u128(0x6ed24bd8_91aa_4b12_ccc7_c97c857ab4e0);
/// The animation the walker starts. The region does not look it up.
pub const STARTED: Uuid = Uuid::from_u128(0xfe770a50_0000_4000_8000_00000000a001);

/// How long each step may take to reach the watcher.
const STEP_WAIT: Duration = Duration::from_secs(10);

pub(super) async fn run(http: &reqwest::Client, harness: &Harness) -> Result<String, String> {
    let (mut walker, mut watcher) =
        enter_observed_pair(http, harness, &harness.owner, &harness.chat).await?;
    let driven = drive(&mut walker, &mut watcher).await;
    let (seen, _) = finish_observed_walk(
        harness,
        watcher,
        &harness.chat,
        walker,
        &harness.owner,
        driven,
        false,
    )
    .await?;
    let walk = steps(&seen.peer_animations)?;
    Ok(format!(
        "animations: pass (the observer saw STAND 1, WALK {walk}, STAND {}, the started animation at {} and its stop)",
        walk.wrapping_add(1),
        walk.wrapping_add(2)
    ))
}

/// Walk, release, then start and stop [`STARTED`], each time until the
/// watcher has the pack.
async fn drive(walker: &mut ObserverSession, watcher: &mut ObserverSession) -> Result<(), String> {
    observed_forward(walker, watcher).await?;
    wait_for(
        walker,
        watcher,
        |list| matches!(list, [(id, _)] if *id == STAND),
    )
    .await?;
    for start in [true, false] {
        let agent = walker.session.agent();
        let message = AgentAnimation {
            agent_data: agent_animation::AgentData {
                agent_id: agent.agent_id,
                session_id: agent.session_id,
            },
            animation_list: vec![agent_animation::AnimationList {
                anim_id: STARTED,
                start_anim: start,
            }],
            ..Default::default()
        };
        walker
            .session
            .send(&message, true)
            .await
            .map_err(|e| e.to_string())?;
        wait_for(walker, watcher, |list| {
            list.iter().any(|(id, _)| *id == STARTED) == start
        })
        .await?;
    }
    Ok(())
}

/// Step both circuits until the watcher's newest pack for the walker passes.
async fn wait_for(
    walker: &mut ObserverSession,
    watcher: &mut ObserverSession,
    done: impl Fn(&[(Uuid, i32)]) -> bool,
) -> Result<(), String> {
    let before = watcher.seen.peer_animations.len();
    let deadline = std::time::Instant::now() + STEP_WAIT;
    loop {
        walker.step().await?;
        watcher.step().await?;
        let seen = &watcher.seen.peer_animations;
        if seen.len() > before && seen.last().is_some_and(|list| done(list)) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "the observer's animation packs for the walker stopped at {seen:?}"
            ));
        }
    }
}

/// The watcher's packs, a repeat folded, must be exactly the five steps.
/// Returns the `WALK` number.
pub fn steps(seen: &[Vec<(Uuid, i32)>]) -> Result<i32, String> {
    let mut packs: Vec<&Vec<(Uuid, i32)>> = Vec::new();
    for list in seen {
        if packs.last() != Some(&list) {
            packs.push(list);
        }
    }
    let walk = match packs.get(1).map(|list| list.as_slice()) {
        Some([(id, sequence)]) if *id == WALK => *sequence,
        _ => return Err(format!("the observer saw no WALK second: {packs:?}")),
    };
    let stand = walk.wrapping_add(1);
    let expected = [
        vec![(STAND, 1)],
        vec![(WALK, walk)],
        vec![(STAND, stand)],
        vec![(STAND, stand), (STARTED, walk.wrapping_add(2))],
        vec![(STAND, stand)],
    ];
    if packs.len() != expected.len()
        || packs
            .iter()
            .zip(&expected)
            .any(|(seen, want)| *seen != want)
    {
        return Err(format!(
            "the observer's animation packs for the walker were {packs:?}, not {expected:?}"
        ));
    }
    Ok(walk)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn five(walk: i32) -> Vec<Vec<(Uuid, i32)>> {
        vec![
            vec![(STAND, 1)],
            vec![(WALK, walk)],
            vec![(WALK, walk)],
            vec![(STAND, walk + 1)],
            vec![(STAND, walk + 1), (STARTED, walk + 2)],
            vec![(STAND, walk + 1)],
        ]
    }

    #[test]
    fn the_five_steps_pass_with_a_repeat_folded() {
        assert_eq!(steps(&five(875_949)), Ok(875_949));
    }

    #[test]
    fn a_wrong_number_or_a_missing_step_fails() {
        let mut skipped = five(10);
        skipped[3] = vec![(STAND, 12)];
        assert!(steps(&skipped).is_err());
        let mut short = five(10);
        short.pop();
        assert!(steps(&short).is_err());
        let mut late = five(10);
        late[4][1].1 = 13;
        assert!(steps(&late).is_err());
        assert!(steps(&[]).is_err());
    }
}
