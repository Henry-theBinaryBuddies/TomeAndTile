//! Checks `fixtures/seeds/pregen-table.json`: the dev seed with the five
//! pregenerated characters on the map and a monster to attack (issue #58).

use std::fs;
use std::path::PathBuf;

use tt_contracts::v1::{
    effect_step, Creature, CreatureKind, HealthState, Snapshot, TargetShape, Token,
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn load<T: serde::de::DeserializeOwned>(relative: &str) -> T {
    let path = fixtures().join(relative);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {relative}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{relative} does not parse: {e}"))
}

fn seed() -> Snapshot {
    load("seeds/pregen-table.json")
}

/// (source file under characters/, creature id in the seed, token id)
const PREGENS: [(&str, &str, &str); 5] = [
    (
        "brannoch-vale-fighter.json",
        "creature-brannoch",
        "token-brannoch",
    ),
    (
        "tamsin-hollowbrook-rogue.json",
        "creature-tamsin",
        "token-tamsin",
    ),
    (
        "korvath-greywind-ranger.json",
        "creature-korvath",
        "token-korvath",
    ),
    (
        "maribel-ashdown-cleric.json",
        "creature-maribel",
        "token-maribel",
    ),
    (
        "ilvaren-quillfeather-wizard.json",
        "creature-ilvaren",
        "token-ilvaren",
    ),
];

fn token<'a>(s: &'a Snapshot, id: &str) -> &'a Token {
    s.tokens
        .iter()
        .find(|t| t.token_id == id)
        .unwrap_or_else(|| panic!("no token {id}"))
}

fn creature<'a>(s: &'a Snapshot, id: &str) -> &'a Creature {
    s.creatures
        .iter()
        .find(|c| c.creature_id == id)
        .unwrap_or_else(|| panic!("no creature {id}"))
}

/// Distance in feet between two tokens on the seed's square grid, using the
/// grid's own cell size and offset and the 5-foot diagonal rule.
fn feet_between(s: &Snapshot, a: &Token, b: &Token) -> f32 {
    let grid = s.scenes[0].grid.as_ref().expect("grid");
    let offset = grid.offset.unwrap_or_default();
    let cell = |t: &Token| {
        let p = t.position.expect("position");
        (
            ((p.x - offset.x) / grid.cell_size).floor(),
            ((p.y - offset.y) / grid.cell_size).floor(),
        )
    };
    let (ax, ay) = cell(a);
    let (bx, by) = cell(b);
    (ax - bx).abs().max((ay - by).abs()) * grid.feet_per_cell
}

fn has_melee_attack(c: &Creature) -> bool {
    c.abilities.iter().any(|a| {
        a.targeting.as_ref().is_some_and(|t| t.range_feet <= 5.0)
            && a.steps
                .iter()
                .any(|s| matches!(s.step, Some(effect_step::Step::Attack(_))))
    })
}

#[test]
fn seed_parses_and_extends_the_mock_snapshot() {
    let seed = seed();
    let mock: Snapshot = load("mock/snapshot.json");
    assert_eq!(seed.members, mock.members);
    assert_eq!(seed.scenes, mock.scenes);
    assert_eq!(seed.active_scene_id, mock.active_scene_id);
    assert_eq!(seed.seq, mock.seq);

    // token-aria is unchanged; token-goblin gains a creature id and HP.
    assert_eq!(token(&seed, "token-aria"), token(&mock, "token-aria"));
    let mut goblin = token(&seed, "token-goblin").clone();
    assert_eq!(goblin.creature_id, "creature-goblin");
    assert_eq!((goblin.hp_current, goblin.hp_max), (Some(9), Some(9)));
    goblin.creature_id.clear();
    goblin.hp_current = None;
    goblin.hp_max = None;
    assert_eq!(&goblin, token(&mock, "token-goblin"));

    // The pregens, plus the ember wisp.
    assert_eq!(seed.tokens.len(), mock.tokens.len() + PREGENS.len() + 1);
    assert_eq!(seed.creatures.len(), PREGENS.len() + 2);
    assert!(seed.tokens.iter().all(|t| !t.hidden), "no token is hidden");
}

#[test]
fn pregen_copies_match_their_source_files() {
    let seed = seed();
    for (file, creature_id, _) in PREGENS {
        let source: Creature = load(&format!("characters/{file}"));
        let mut copy = creature(&seed, creature_id).clone();
        assert!(!copy.creature_id.is_empty());
        copy.creature_id.clear();
        assert_eq!(
            copy, source,
            "{file}: the seed copy differs from its source"
        );
    }
}

#[test]
fn every_pregen_has_a_linked_token_for_mock_player() {
    let seed = seed();
    let scene = &seed.scenes[0];
    for (file, creature_id, token_id) in PREGENS {
        let c = creature(&seed, creature_id);
        let t = token(&seed, token_id);
        assert_eq!(t.creature_id, creature_id, "{file}");
        assert_eq!(t.scene_id, scene.scene_id, "{file}");
        assert_eq!(t.level_id, scene.levels[0].level_id, "{file}");
        assert_eq!(t.controller_user_ids, ["mock-player"], "{file}");
        assert_eq!(t.hp_current, Some(c.hp_current), "{file}");
        assert_eq!(t.hp_max, Some(c.hp_max), "{file}");
        assert_eq!(t.health_state, HealthState::Healthy as i32, "{file}");
    }
}

#[test]
fn the_goblin_and_the_ember_wisp_are_the_only_monsters() {
    let seed = seed();
    let monsters: Vec<&Creature> = seed
        .creatures
        .iter()
        .filter(|c| c.kind == CreatureKind::Monster as i32)
        .collect();
    assert_eq!(monsters.len(), 2);
    let m = creature(&seed, "creature-goblin");
    assert!(m.armor_class > 0 && m.hp_max > 0 && m.hp_current == m.hp_max);
    assert_eq!(m.abilities.len(), 1);
    let a = &m.abilities[0];
    let targeting = a.targeting.as_ref().expect("targeting");
    assert_eq!(targeting.shape, TargetShape::Creatures as i32);
    assert_eq!(a.steps.len(), 1);
    assert!(matches!(
        a.steps[0].step,
        Some(effect_step::Step::Attack(_))
    ));
}

#[test]
fn tokens_are_placed_for_in_reach_and_out_of_reach_attacks() {
    let seed = seed();
    let goblin = token(&seed, "token-goblin");
    assert_eq!(
        feet_between(&seed, token(&seed, "token-brannoch"), goblin),
        5.0,
        "Brannoch's longsword reaches the goblin"
    );
    let out_of_reach_melee = PREGENS.iter().any(|(_, cid, tid)| {
        has_melee_attack(creature(&seed, cid))
            && feet_between(&seed, token(&seed, tid), goblin) > 5.0
    });
    assert!(
        out_of_reach_melee,
        "some pregen with a melee attack stands beyond 5 feet of the goblin"
    );

    // No two tokens share a cell.
    let cell_size = seed.scenes[0].grid.as_ref().expect("grid").cell_size;
    let mut cells: Vec<(i64, i64)> = seed
        .tokens
        .iter()
        .map(|t| {
            let p = t.position.expect("position");
            (
                (p.x / cell_size).floor() as i64,
                (p.y / cell_size).floor() as i64,
            )
        })
        .collect();
    cells.sort_unstable();
    let before = cells.len();
    cells.dedup();
    assert_eq!(cells.len(), before, "two tokens share a grid cell");
}
