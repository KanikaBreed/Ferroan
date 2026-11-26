// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One row per live scenario.
//!
//! `cargo xtask live` reads the row for flag parsing, help text, the Ferroan
//! allow-list, the parcel seed, the Docker policy and which comparison runs.
//! The bot reads the same row for the scenario it executes. A new scenario
//! adds a row here.

use std::sync::OnceLock;

/// Which `grid` / `--region ferroan` / `--physics basicphysics` shapes parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flags {
    /// Any combination is a run. `--running` is usage.
    Any,
    /// `assets`: `--running` alone keeps the services up. Other shapes are runs.
    Assets,
    /// `grid --physics basicphysics`, and not `--region ferroan`.
    Pin8,
    /// The Pin8 shape, or `grid --region ferroan` with or without basicphysics.
    /// `--region ferroan` without `grid` is usage.
    Pin8OrFerroan,
    /// The Pin8 shape, or `grid --region ferroan` without basicphysics.
    ParcelTiming,
    /// `grid --physics basicphysics`, with or without `--region ferroan`.
    /// The neighbour stays C#.
    ChildView,
}

/// Whether `grid --region ferroan` may run this scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ferroan {
    /// Allowed. `ord` is the place in the refusal sentence.
    Allow(u8),
    /// Required. The string is the error when the region is C#.
    Require(u8, &'static str),
    Deny,
}

impl Ferroan {
    const fn ord(self) -> u8 {
        match self {
            Ferroan::Allow(ord) | Ferroan::Require(ord, _) => ord,
            Ferroan::Deny => 0,
        }
    }

    pub const fn require_message(self) -> Option<&'static str> {
        match self {
            Ferroan::Require(_, message) => Some(message),
            Ferroan::Allow(_) | Ferroan::Deny => None,
        }
    }
}

/// What the bot executes for this name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bot {
    Login,
    Walk,
    Chat,
    Build,
    Teleport,
    Texture,
    Inventory,
    All,
    Assets,
    Caps,
    Neighbour,
    Hop,
    Roots,
    Uploads,
    ObjectCross,
    PrimArrival,
    MaterialOverride,
    Stored(Stored),
    Avatars,
    ChildView,
    ParcelTiming,
    ParcelProperties,
    ParcelOverlay,
    Limits,
    Report,
    Ferrobot,
}

/// Stored-prim viewer runs. The harness compares the capture afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stored {
    Capture,
    Handoff,
    Load,
}

/// What the grid harness compares after the bot returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Login,
    ParcelTiming,
    ParcelProperties,
    ObjectCross,
    PrimCapture,
    PrimArrival,
    MaterialOverride,
    Report,
    Ferrobot,
    /// Ferroan prints the proxy report. C# finishes the capture.
    Proxy,
    TwoRoots,
    Avatars,
    Chat,
    Teleport,
    Neighbour,
    /// A Ferroan run compares the login replies. A C# run finishes the capture.
    /// Walk, texture, inventory, assets and caps use this.
    Region,
    /// A C# run finishes the capture, as [`Compare::Region`] does on C#.
    /// Build, `all`, store-handoff, prim-load and the parcel overlays use
    /// this. Those rows deny the Ferroan region. Child-view uses it too, and
    /// its home region may be Ferroan.
    Finish,
}

/// Parcel overlay installed on the home region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    Empty,
    Multi,
    Lone,
    Legacy,
}

/// Region-store fixture the home or neighbour region opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Standard,
    EmptyPrim,
    Prim,
    EditPrim,
    Load,
}

/// Who drives the scenario once both regions are up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    Bot,
    Ferrobot,
    Limits,
    /// `store-handoff` and `prim-load` share the home SQLite file and refuse Docker.
    Handoff,
}

/// Names the grid harness logs in, when that differs from one bot call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sequence {
    Default,
    Append(&'static [&'static str]),
    Replace(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scenario {
    pub name: &'static str,
    pub flags: Flags,
    pub ferroan: Ferroan,
    pub bot: Bot,
    pub compare: Compare,
    pub runner: Runner,
    pub seed: Option<Seed>,
    pub home_store: Store,
    pub neighbour_store: Store,
    pub sequence: Sequence,
    pub help_at: u16,
    pub help: &'static str,
    pub in_all: bool,
    pub in_assets: bool,
    /// Standalone `live run` accepts this name.
    pub standalone: bool,
    pub login_only: bool,
    pub metrics_always: bool,
    pub patch_materials: bool,
    pub relay: bool,
    pub login_probe: bool,
    pub inventory_probe: bool,
    pub smoke_logs: bool,
    pub pass_metrics: bool,
    pub neighbour_terrain: bool,
    pub settle_second: bool,
    pub observe_login: bool,
    pub groups_always: bool,
}

const fn row(
    name: &'static str,
    flags: Flags,
    ferroan: Ferroan,
    bot: Bot,
    compare: Compare,
) -> Scenario {
    Scenario {
        name,
        flags,
        ferroan,
        bot,
        compare,
        runner: Runner::Bot,
        seed: None,
        home_store: Store::Standard,
        neighbour_store: Store::Standard,
        sequence: Sequence::Default,
        help_at: 0,
        help: "",
        in_all: false,
        in_assets: false,
        standalone: false,
        login_only: false,
        metrics_always: false,
        patch_materials: false,
        relay: false,
        login_probe: false,
        inventory_probe: false,
        smoke_logs: false,
        pass_metrics: false,
        neighbour_terrain: false,
        settle_second: false,
        observe_login: false,
        groups_always: false,
    }
}

impl Scenario {
    const fn in_all(mut self) -> Self {
        self.in_all = true;
        self
    }
    const fn in_assets(mut self) -> Self {
        self.in_assets = true;
        self
    }
    const fn standalone(mut self) -> Self {
        self.standalone = true;
        self
    }
    const fn login_only(mut self) -> Self {
        self.login_only = true;
        self
    }
    const fn metrics_always(mut self) -> Self {
        self.metrics_always = true;
        self
    }
    const fn patch_materials(mut self) -> Self {
        self.patch_materials = true;
        self
    }
    const fn relay(mut self) -> Self {
        self.relay = true;
        self
    }
    const fn login_probe(mut self) -> Self {
        self.login_probe = true;
        self
    }
    const fn inventory_probe(mut self) -> Self {
        self.inventory_probe = true;
        self
    }
    const fn smoke_logs(mut self) -> Self {
        self.smoke_logs = true;
        self
    }
    const fn pass_metrics(mut self) -> Self {
        self.pass_metrics = true;
        self
    }
    const fn neighbour_terrain(mut self) -> Self {
        self.neighbour_terrain = true;
        self
    }
    const fn settle_second(mut self) -> Self {
        self.settle_second = true;
        self
    }
    const fn observe_login(mut self) -> Self {
        self.observe_login = true;
        self
    }
    const fn groups_always(mut self) -> Self {
        self.groups_always = true;
        self
    }
    const fn seed(mut self, seed: Seed) -> Self {
        self.seed = Some(seed);
        self
    }
    const fn runner(mut self, runner: Runner) -> Self {
        self.runner = runner;
        self
    }
    const fn home(mut self, store: Store) -> Self {
        self.home_store = store;
        self
    }
    const fn neighbour(mut self, store: Store) -> Self {
        self.neighbour_store = store;
        self
    }
    const fn sequence(mut self, sequence: Sequence) -> Self {
        self.sequence = sequence;
        self
    }
    const fn help(mut self, at: u16, text: &'static str) -> Self {
        self.help_at = at;
        self.help = text;
        self
    }
}

const HELP_OBJECT_CROSS: &str = concat!(
    "       cargo xtask live run object-cross grid --physics basicphysics\n",
    "       cargo xtask live object-cross-fixtures <capture-dir>\n",
);

const HELP_PARCEL_TIMING: &str = concat!(
    "       cargo xtask live run parcel-timing grid --physics basicphysics\n",
    "       cargo xtask live run parcel-timing grid --region ferroan\n",
    "       cargo xtask live parcel-timing-fixtures <capture-dir>\n",
);

const HELP_PARCEL_PROPERTIES: &str = concat!(
    "       cargo xtask live run parcel-properties grid --physics basicphysics\n",
    "       cargo xtask live parcel-properties-fixtures <capture-dir>\n",
);

const HELP_PARCELS: &str = "       cargo xtask live run parcel-empty|parcel-multi|parcel-lone|parcel-legacy grid --physics basicphysics\n";

const HELP_PRIM_LOAD: &str = "       cargo xtask live run prim-load grid --physics basicphysics\n";

const HELP_STORE_HANDOFF: &str =
    "       cargo xtask live run store-handoff grid --physics basicphysics\n";

const HELP_PRIM_CAPTURE: &str =
    "       cargo xtask live run prim-capture grid --physics basicphysics\n";

const HELP_CHILD_VIEW: &str = concat!(
    "       cargo xtask live run child-view grid --physics basicphysics\n",
    "       cargo xtask live run child-view grid --region ferroan --physics basicphysics\n",
);

const HELP_PRIM_ARRIVAL: &str = concat!(
    "       cargo xtask live run prim-arrival grid --region ferroan\n",
    "       cargo xtask live run prim-arrival grid --physics basicphysics\n",
);

const HELP_MATERIAL: &str = concat!(
    "       cargo xtask live run material-override grid --physics basicphysics\n",
    "       cargo xtask live run material-override grid --region ferroan\n",
);

const HELP_ALL: &str = "       cargo xtask live run <scenario>|all [grid] [--region ferroan] [--physics basicphysics] [--rust <service>,…] [--db sqlite|mysql|pgsql]\n";

const HELP_LOGIN: &str = "       cargo xtask live run login grid --region ferroan\n";

const HELP_LIMITS: &str = "       cargo xtask live run limits grid --region ferroan\n";

const HELP_CAPS: &str = "       cargo xtask live run caps grid [--region ferroan]\n";

const HELP_FERROBOT: &str = "       cargo xtask live run ferrobot [grid [--region ferroan]]\n";

const HELP_ASSETS: &str = concat!(
    "       cargo xtask live run assets grid --region ferroan\n",
    "       cargo xtask live run <scenario>|all grid --rust assets [--db sqlite]\n",
    "       cargo xtask live run login grid --rust accounts\n",
    "       cargo xtask live run login grid --rust auth\n",
    "       cargo xtask live run login grid --rust assets,accounts\n",
    "       cargo xtask live caps-fixtures <capture-dir>\n",
    "       cargo xtask live login-fixtures <capture-dir>\n",
    "       cargo xtask live region-parity-fixtures <capture-dir>\n",
    "       cargo xtask live root-arrival-fixtures <capture-dir>\n",
    "       cargo xtask live circuit-fixtures <udp-capture>\n",
    "       cargo xtask live arrival-fixtures <udp-capture>\n",
    "       cargo xtask live trailing-fixtures <udp-capture>\n",
    "       cargo xtask live run assets --running\n",
);

const LIMITS_REGION: &str = "the limits scenario runs on the Ferroan region";

/// `in_all` rows stay in the order `all` runs them. `texture` and `inventory`
/// are the asset pair, so they stay last among those seven.
pub const SCENARIOS: &[Scenario] = &[
    row(
        "login",
        Flags::Any,
        Ferroan::Allow(1),
        Bot::Login,
        Compare::Login,
    )
    .in_all()
    .standalone()
    .login_only()
    .login_probe()
    .smoke_logs()
    .observe_login()
    .help(120, HELP_LOGIN),
    row(
        "walk",
        Flags::Any,
        Ferroan::Allow(15),
        Bot::Walk,
        Compare::Region,
    )
    .in_all()
    .standalone(),
    row(
        "chat",
        Flags::Any,
        Ferroan::Allow(9),
        Bot::Chat,
        Compare::Chat,
    )
    .in_all()
    .standalone()
    .settle_second(),
    row(
        "build",
        Flags::Any,
        Ferroan::Deny,
        Bot::Build,
        Compare::Finish,
    )
    .in_all()
    .standalone(),
    row(
        "teleport",
        Flags::Any,
        Ferroan::Allow(10),
        Bot::Teleport,
        Compare::Teleport,
    )
    .in_all()
    .standalone(),
    row(
        "texture",
        Flags::Any,
        Ferroan::Allow(4),
        Bot::Texture,
        Compare::Region,
    )
    .in_all()
    .in_assets()
    .standalone()
    .inventory_probe(),
    row(
        "inventory",
        Flags::Any,
        Ferroan::Allow(5),
        Bot::Inventory,
        Compare::Region,
    )
    .in_all()
    .in_assets()
    .standalone()
    .inventory_probe(),
    row(
        "assets",
        Flags::Assets,
        Ferroan::Allow(6),
        Bot::Assets,
        Compare::Region,
    )
    .inventory_probe()
    .standalone()
    .help(160, HELP_ASSETS),
    row("all", Flags::Any, Ferroan::Deny, Bot::All, Compare::Finish)
        .sequence(Sequence::Append(&["neighbour"]))
        .standalone()
        .help(110, HELP_ALL),
    row(
        "caps",
        Flags::Any,
        Ferroan::Allow(2),
        Bot::Caps,
        Compare::Region,
    )
    .login_only()
    .help(140, HELP_CAPS),
    row(
        "ferrobot",
        Flags::Any,
        Ferroan::Allow(3),
        Bot::Ferrobot,
        Compare::Ferrobot,
    )
    .runner(Runner::Ferrobot)
    .login_only()
    .relay()
    .help(150, HELP_FERROBOT),
    row(
        "limits",
        Flags::Any,
        Ferroan::Require(7, LIMITS_REGION),
        Bot::Limits,
        Compare::Proxy,
    )
    .runner(Runner::Limits)
    .help(130, HELP_LIMITS),
    row(
        "roots",
        Flags::Any,
        Ferroan::Allow(8),
        Bot::Roots,
        Compare::TwoRoots,
    )
    .pass_metrics()
    .settle_second(),
    row(
        "hop",
        Flags::Any,
        Ferroan::Allow(11),
        Bot::Hop,
        Compare::Proxy,
    ),
    row(
        "uploads",
        Flags::Any,
        Ferroan::Allow(12),
        Bot::Uploads,
        Compare::TwoRoots,
    )
    .settle_second()
    .standalone(),
    row(
        "avatars",
        Flags::Any,
        Ferroan::Allow(13),
        Bot::Avatars,
        Compare::Avatars,
    )
    .settle_second(),
    row(
        "neighbour",
        Flags::Any,
        Ferroan::Allow(14),
        Bot::Neighbour,
        Compare::Neighbour,
    )
    .neighbour_terrain(),
    row(
        "report",
        Flags::Any,
        Ferroan::Allow(19),
        Bot::Report,
        Compare::Report,
    )
    .sequence(Sequence::Replace(&[
        "login",
        "chat",
        "teleport",
        "neighbour",
    ])),
    row(
        "prim-capture",
        Flags::Pin8,
        Ferroan::Deny,
        Bot::Stored(Stored::Capture),
        Compare::PrimCapture,
    )
    .home(Store::Prim)
    .help(70, HELP_PRIM_CAPTURE),
    row(
        "prim-arrival",
        Flags::Pin8OrFerroan,
        Ferroan::Allow(17),
        Bot::PrimArrival,
        Compare::PrimArrival,
    )
    .home(Store::Prim)
    .help(90, HELP_PRIM_ARRIVAL),
    row(
        "material-override",
        Flags::Pin8OrFerroan,
        Ferroan::Allow(18),
        Bot::MaterialOverride,
        Compare::MaterialOverride,
    )
    .home(Store::Prim)
    .patch_materials()
    .help(100, HELP_MATERIAL),
    row(
        "object-cross",
        Flags::Pin8,
        Ferroan::Deny,
        Bot::ObjectCross,
        Compare::ObjectCross,
    )
    .home(Store::Prim)
    .neighbour(Store::EmptyPrim)
    .settle_second()
    .help(10, HELP_OBJECT_CROSS),
    row(
        "child-view",
        Flags::ChildView,
        Ferroan::Allow(16),
        Bot::ChildView,
        Compare::Finish,
    )
    .neighbour(Store::Prim)
    .help(80, HELP_CHILD_VIEW),
    row(
        "store-handoff",
        Flags::Pin8,
        Ferroan::Deny,
        Bot::Stored(Stored::Handoff),
        Compare::Finish,
    )
    .runner(Runner::Handoff)
    .home(Store::EditPrim)
    .help(60, HELP_STORE_HANDOFF),
    row(
        "prim-load",
        Flags::Pin8,
        Ferroan::Deny,
        Bot::Stored(Stored::Load),
        Compare::Finish,
    )
    .runner(Runner::Handoff)
    .home(Store::Load)
    .metrics_always()
    .help(50, HELP_PRIM_LOAD),
    row(
        "parcel-timing",
        Flags::ParcelTiming,
        Ferroan::Allow(20),
        Bot::ParcelTiming,
        Compare::ParcelTiming,
    )
    .seed(Seed::Multi)
    .groups_always()
    .help(20, HELP_PARCEL_TIMING),
    row(
        "parcel-properties",
        Flags::Pin8,
        Ferroan::Deny,
        Bot::ParcelProperties,
        Compare::ParcelProperties,
    )
    .seed(Seed::Multi)
    .help(30, HELP_PARCEL_PROPERTIES),
    row(
        "parcel-empty",
        Flags::Any,
        Ferroan::Deny,
        Bot::ParcelOverlay,
        Compare::Finish,
    )
    .seed(Seed::Empty)
    .help(40, HELP_PARCELS),
    row(
        "parcel-multi",
        Flags::Any,
        Ferroan::Deny,
        Bot::ParcelOverlay,
        Compare::Finish,
    )
    .seed(Seed::Multi),
    row(
        "parcel-lone",
        Flags::Any,
        Ferroan::Deny,
        Bot::ParcelOverlay,
        Compare::Finish,
    )
    .seed(Seed::Lone),
    row(
        "parcel-legacy",
        Flags::Any,
        Ferroan::Deny,
        Bot::ParcelOverlay,
        Compare::Finish,
    )
    .seed(Seed::Legacy),
];

pub fn get(name: &str) -> Option<&'static Scenario> {
    SCENARIOS.iter().find(|row| row.name == name)
}

/// The scenarios `all` runs, in order.
pub fn names() -> &'static [&'static str] {
    static CACHE: OnceLock<Vec<&'static str>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            SCENARIOS
                .iter()
                .filter(|row| row.in_all)
                .map(|row| row.name)
                .collect()
        })
        .as_slice()
}

fn asset_names() -> &'static [&'static str] {
    static CACHE: OnceLock<Vec<&'static str>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            SCENARIOS
                .iter()
                .filter(|row| row.in_assets)
                .map(|row| row.name)
                .collect()
        })
        .as_slice()
}

pub fn selection(name: &str) -> Option<&'static [&'static str]> {
    let row = get(name)?;
    if row.bot == Bot::All {
        return Some(names());
    }
    if row.bot == Bot::Assets {
        return Some(asset_names());
    }
    if row.in_all {
        let names = names();
        let index = names.iter().position(|item| *item == name)?;
        return Some(&names[index..index + 1]);
    }
    None
}

/// Standalone `live run` accepts the rows marked [`Scenario::standalone`].
pub fn runs_standalone(name: &str) -> bool {
    get(name).is_some_and(|row| row.standalone)
}

/// Logins the grid harness performs for one `live run` name.
pub fn grid_names(scenario: &str) -> Option<Vec<&'static str>> {
    let row = get(scenario)?;
    let names = match row.sequence {
        Sequence::Replace(names) => names.to_vec(),
        Sequence::Append(extra) => {
            let mut names = one_run(row);
            names.extend_from_slice(extra);
            names
        }
        Sequence::Default => one_run(row),
    };
    Some(names)
}

fn one_run(row: &Scenario) -> Vec<&'static str> {
    if row.bot == Bot::All {
        return names().to_vec();
    }
    if row.bot == Bot::Assets {
        return asset_names().to_vec();
    }
    vec![row.name]
}

/// Memberships come from Robust for parcel timing, and for a Ferroan parcel seed.
pub fn remote_groups(row: &Scenario, ferroan: bool) -> bool {
    row.groups_always || (ferroan && row.seed.is_some())
}

pub fn live_run_help() -> String {
    let mut rows: Vec<&Scenario> = SCENARIOS.iter().filter(|row| row.help_at > 0).collect();
    rows.sort_by_key(|row| row.help_at);
    let mut text = String::new();
    for row in rows {
        text.push_str(row.help);
    }
    text
}

pub fn ferroan_denied() -> String {
    let mut rows: Vec<&Scenario> = SCENARIOS
        .iter()
        .filter(|row| row.ferroan.ord() > 0)
        .collect();
    rows.sort_by_key(|row| row.ferroan.ord());
    let names: Vec<&str> = rows.iter().map(|row| row.name).collect();
    let list = match names.as_slice() {
        [only] => (*only).to_string(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
        [] => String::new(),
    };
    format!("the Ferroan grid region currently supports {list} scenarios")
}

/// Frozen wording. The three sentences below keep the grouped parcel form and
/// the prim-capture parenthetical. A new scenario row leaves them as they are.
pub fn grid_unknown(scenario: &str) -> String {
    format!(
        "unknown scenario {scenario}; expected all, neighbour, object-cross, child-view, prim-load, store-handoff, parcel-{{empty,multi,lone,legacy,timing,properties}}, prim-arrival, material-override, prim-capture (live run prim-capture grid --physics basicphysics) or {}",
        names().join(", ")
    )
}

pub fn bot_unknown(name: &str) -> String {
    format!(
        "unknown scenario {name}; expected all, object-cross, prim-load, store-handoff, prim-arrival, material-override, prim-capture (live run prim-capture grid --physics basicphysics) or {}",
        names().join(", ")
    )
}

pub fn standalone_unknown(scenario: &str) -> String {
    format!(
        "unknown scenario {scenario}; expected all, uploads or {}",
        names().join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_stay_in_all_order_and_assets_are_the_last_pair() {
        assert_eq!(
            names(),
            &[
                "login",
                "walk",
                "chat",
                "build",
                "teleport",
                "texture",
                "inventory"
            ]
        );
        assert_eq!(asset_names(), &["texture", "inventory"]);
        assert_eq!(selection("all").unwrap(), names());
        assert_eq!(selection("login").unwrap(), &["login"]);
        assert_eq!(selection("assets").unwrap(), &["texture", "inventory"]);
        assert!(selection("neighbour").is_none());
        assert!(selection("capture").is_none());
    }

    #[test]
    fn each_name_is_registered_once() {
        let mut seen = Vec::new();
        for row in SCENARIOS {
            assert!(!seen.contains(&row.name), "{}", row.name);
            seen.push(row.name);
            if row.help_at > 0 {
                assert!(row.help.ends_with('\n'), "{}", row.name);
                assert_eq!(
                    SCENARIOS
                        .iter()
                        .filter(|other| other.help_at == row.help_at)
                        .count(),
                    1,
                    "{}",
                    row.name
                );
            }
        }
    }

    #[test]
    fn grid_sequences_keep_all_and_report() {
        let mut all = names().to_vec();
        all.push("neighbour");
        assert_eq!(grid_names("all").unwrap(), all);
        assert_eq!(
            grid_names("report").unwrap(),
            vec!["login", "chat", "teleport", "neighbour"]
        );
        assert_eq!(grid_names("assets").unwrap(), vec!["texture", "inventory"]);
        assert_eq!(grid_names("caps").unwrap(), vec!["caps"]);
        assert!(grid_names("not-a-scenario").is_none());
    }

    #[test]
    fn every_row_keeps_its_registered_fields() {
        let actual = SCENARIOS
            .iter()
            .map(|row| format!("{row:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        let expected = include_str!("register_rows.snap");
        assert_eq!(
            actual,
            expected.trim_end_matches('\n'),
            "a scenario row field changed"
        );
    }

    #[test]
    fn ferroan_refusal_lists_the_allowed_rows_in_order() {
        assert_eq!(
            ferroan_denied(),
            "the Ferroan grid region currently supports login, caps, ferrobot, texture, inventory, assets, limits, roots, chat, teleport, hop, uploads, avatars, neighbour, walk, child-view, prim-arrival, material-override, report and parcel-timing scenarios"
        );
    }
}
