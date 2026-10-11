//! Synthetic item rules shared by the session tests (no game data).

use std::sync::Arc;

use sn_session::items::{ItemRules, OutcropDrop, OutcropInfo, PickupInfo};
use sn_sim::inventory::{AnalysisEntry, SlotTable, Techs};

pub const QUARTZ: i32 = 1;
pub const BIG: i32 = 2;
pub const TANK: i32 = 30;
pub const FOOD: i32 = 40;

pub fn rules() -> Arc<ItemRules> {
    let mut techs = Techs::default();
    techs.sizes.insert(BIG, (2, 2));
    techs.sizes.insert(TANK, (2, 3));
    techs.equipment.insert(TANK, 2);
    let pickup = |tech, eatable| PickupInfo {
        tech,
        pickupable: true,
        destroy_on_death: true,
        eatable,
    };
    let mut r = ItemRules {
        techs,
        slots: SlotTable {
            slots: [("Head", 1), ("Tank", 2), ("Chip1", 3), ("Chip2", 3)]
                .iter()
                .map(|&(n, t)| (n.to_owned(), t))
                .collect(),
            vehicle_module: 9,
            seamoth_module: 13,
            exosuit_module: 14,
        },
        player_slots: vec!["Head".into(), "Tank".into(), "Chip1".into()],
        inventory_size: (6, 8),
        analysis: vec![AnalysisEntry {
            tech: QUARTZ,
            unlocks: vec![100],
        }],
        default_tech: vec![500],
        locker_size: (4, 8),
        supplies: vec![FOOD, FOOD, TANK],
        luck: vec![(QUARTZ, 0.0), (BIG, 0.0)],
        ..Default::default()
    };
    r.pickups.insert("quartz".into(), pickup(QUARTZ, false));
    r.pickups.insert("big".into(), pickup(BIG, false));
    r.pickups.insert("tank".into(), pickup(TANK, false));
    r.pickups.insert("food".into(), pickup(FOOD, true));
    r.pickups.insert(
        "prop".into(),
        PickupInfo {
            pickupable: false,
            ..pickup(5, false)
        },
    );
    r.outcrops.insert(
        "lime".into(),
        OutcropInfo {
            hits_to_break: 3,
            num_chances: 1,
            list: vec![
                OutcropDrop {
                    tech: QUARTZ,
                    chance: 0.5,
                    class: "quartz".into(),
                },
                OutcropDrop {
                    tech: BIG,
                    chance: 0.25,
                    class: "big".into(),
                },
            ],
            default_class: "quartz".into(),
        },
    );
    r.tech_class.insert(FOOD, "food".into());
    r.tech_class.insert(TANK, "tank".into());
    Arc::new(r)
}
