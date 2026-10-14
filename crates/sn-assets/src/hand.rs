//! The hand on items (M9d5; here since M9d6a, so that `sn-client` and
//! `sn-inspect` share it): what `HandReticle` shows over a pickup, an
//! outcrop or the lifepod's locker, and what a click asks the server, as
//! the game's `OnHandHover` / `OnHandClick` of those objects do.

use sn_protocol::{EntityKey, HitKind, Request};
use sn_sim::V3;

use sn_ui::reticle::IconType;

use crate::{ItemText, TargetBody, TargetKind};

/// Language keys the game's code passes as literals (M9d5): the pick-up
/// text (`LanguageCache.GetPickupText`), the full inventory
/// (`Pickupable.OnHandHover`), the lifepod's locker
/// (`EscapePod.StorageHover`).
const PICK_UP_FORMAT: &str = "PickUpFormat";
/// `LanguageCache.GetPackUpText` (M9u6d2).
const PACK_UP_FORMAT: &str = "PackUpFormat";
const INVENTORY_FULL: &str = "InventoryFull";
const POD_STORAGE: &str = "EscapePodStorageOpen";

/// What the hand is on, as the game's `OnHandHover` / `OnHandClick` of
/// that object see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Aim {
    /// `HandReticle`'s hand text and its subscript.
    pub text: String,
    pub subscript: String,
    /// The hand text ends in the left hand's button (`SetText(…,
    /// GameInput.Button.LeftHand)`; M9u6d2).
    pub button: bool,
    /// `SetIcon`'s icon (M9u6d2); `None`: not called (`Default`).
    pub icon: Option<IconType>,
    pub action: AimAction,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AimAction {
    /// A click asks for this, and plays the grab (`Player.PlayGrab`).
    Request(Request),
    /// A click opens the lifepod's locker (`EscapePod.StorageOpen`).
    OpenLocker,
    /// A click does nothing (no room, not pickupable).
    None,
}

/// The hand on a target (a pickup or an outcrop) or on the lifepod's
/// locker. `room`: whether the own inventory has room for a tech type
/// (`Player.HasInventoryRoom`).
pub fn aim(
    text: &ItemText,
    rules: &sn_sim::inventory::ItemRules,
    target: Option<&TargetBody>,
    locker: bool,
    room: impl Fn(i32) -> bool,
) -> Option<Aim> {
    if locker {
        return Some(Aim {
            text: text.get(POD_STORAGE).to_string(),
            subscript: String::new(),
            button: true,
            icon: Some(IconType::Hand),
            action: AimAction::OpenLocker,
        });
    }
    let t = target?;
    // A server-made object's class is the server's to say.
    let class = match t.key {
        EntityKey::Spawned(_) => String::new(),
        _ => t.class.clone(),
    };
    match t.kind {
        TargetKind::Pickup => {
            let info = rules.pickups.get(&t.class)?;
            let name = text.tech(info.tech);
            // `AllowedToPickUp`: pickupable and room (the 1 s after a drop
            // always passes, see `sn_sim::inventory::pickup`).
            Some(if info.pickupable && room(info.tech) {
                // `usePackUpIcon`: the pack-up text and icon.
                let (format, icon) = if info.pack_up_icon {
                    (PACK_UP_FORMAT, IconType::PackUp)
                } else {
                    (PICK_UP_FORMAT, IconType::Hand)
                };
                Aim {
                    text: text.format(format, &name),
                    subscript: String::new(),
                    button: true,
                    icon: Some(icon),
                    action: AimAction::Request(Request::PickUp {
                        key: t.key.clone(),
                        class,
                    }),
                }
            } else if info.pickupable {
                Aim {
                    text: name,
                    subscript: text.get(INVENTORY_FULL).to_string(),
                    button: false,
                    icon: None,
                    action: AimAction::None,
                }
            } else {
                Aim {
                    text: name,
                    subscript: String::new(),
                    button: false,
                    icon: None,
                    action: AimAction::None,
                }
            })
        }
        TargetKind::Outcrop => {
            let info = rules.outcrops.get(&t.class)?;
            // `BreakIntoResources` spawns at `position + up ×
            // verticalSpawnOffset`.
            let up = sn_sim::Q::from_f32(t.at.rotation).rotate(V3::new(0.0, 1.0, 0.0));
            let spawn = sn_sim::inventory::Outcrop::spawn_point(
                V3::from_f32(t.at.position),
                up,
                f64::from(info.vertical_offset),
            );
            Some(Aim {
                text: text.get(&info.break_text).to_string(),
                subscript: String::new(),
                button: true,
                icon: Some(IconType::Hand),
                action: AimAction::Request(Request::Hit {
                    key: t.key.clone(),
                    class,
                    // No tool held: `PlayBash` is false, so the hand hits.
                    kind: HitKind::Hand,
                    spawn: spawn.to_f32(),
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sn_protocol::ItemId;
    use sn_sim::inventory::{ItemRules, OutcropInfo, PickupInfo};
    use sn_world::Transform as Placement;

    use super::*;

    fn text() -> ItemText {
        ItemText {
            tech_names: HashMap::from([(5, "Titanium".to_string()), (7, "Limestone".to_string())]),
            strings: HashMap::from([
                ("Titanium".to_string(), "Ti".to_string()),
                ("PickUpFormat".to_string(), "Pick up {0}".to_string()),
                ("PackUpFormat".to_string(), "Pack up {0}".to_string()),
                ("InventoryFull".to_string(), "Full".to_string()),
                ("BreakLime".to_string(), "Break it".to_string()),
                ("EscapePodStorageOpen".to_string(), "Open".to_string()),
            ]),
        }
    }

    fn rules() -> ItemRules {
        let mut r = ItemRules::default();
        let pickup = |tech, pickupable| PickupInfo {
            tech,
            pickupable,
            destroy_on_death: true,
            eatable: false,
            randomize_rotation: true,
            pack_up_icon: false,
        };
        r.pickups.insert("ti".into(), pickup(5, true));
        r.pickups.insert("stuck".into(), pickup(5, false));
        r.outcrops.insert(
            "lime".into(),
            OutcropInfo {
                hits_to_break: 3,
                vertical_offset: 0.5,
                num_chances: 1,
                list: Vec::new(),
                default_class: "ti".into(),
                break_text: "BreakLime".into(),
            },
        );
        r
    }

    fn target(class: &str, kind: TargetKind, key: EntityKey) -> TargetBody {
        TargetBody {
            key,
            class: class.into(),
            kind,
            at: Placement {
                position: [1.0, 2.0, 3.0],
                // 90° about z: up becomes −x.
                rotation: [
                    0.0,
                    0.0,
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                ],
                scale: [1.0; 3],
            },
        }
    }

    #[test]
    fn the_hand_on_items_says_and_asks_what_the_game_does() {
        let (t, r) = (text(), rules());
        let placed = EntityKey::Placed("a".into());
        let ti = target("ti", TargetKind::Pickup, placed.clone());

        let a = aim(&t, &r, Some(&ti), false, |_| true).unwrap();
        assert_eq!((a.text.as_str(), a.subscript.as_str()), ("Pick up Ti", ""));
        assert_eq!((a.button, a.icon), (true, Some(IconType::Hand)));
        assert_eq!(
            a.action,
            AimAction::Request(Request::PickUp {
                key: placed.clone(),
                class: "ti".into()
            })
        );

        // No room: the name and "inventory full", no request.
        let a = aim(&t, &r, Some(&ti), false, |_| false).unwrap();
        assert_eq!((a.text.as_str(), a.subscript.as_str()), ("Ti", "Full"));
        assert_eq!(a.action, AimAction::None);
        // No icon set (it stays `Default`), no button.
        assert_eq!((a.button, a.icon), (false, None));

        // Not pickupable: the name only.
        let stuck = target("stuck", TargetKind::Pickup, placed.clone());
        let a = aim(&t, &r, Some(&stuck), false, |_| true).unwrap();
        assert_eq!((a.text.as_str(), a.subscript.as_str()), ("Ti", ""));
        assert_eq!(a.action, AimAction::None);

        // A server-made object: the server knows its class.
        let spawned = target("ti", TargetKind::Pickup, EntityKey::Spawned(ItemId(9)));
        let a = aim(&t, &r, Some(&spawned), false, |_| true).unwrap();
        assert_eq!(
            a.action,
            AimAction::Request(Request::PickUp {
                key: EntityKey::Spawned(ItemId(9)),
                class: String::new()
            })
        );

        // An outcrop: its break text; a hand hit, resources at
        // position + up × verticalSpawnOffset.
        let lime = target("lime", TargetKind::Outcrop, placed.clone());
        let a = aim(&t, &r, Some(&lime), false, |_| true).unwrap();
        assert_eq!(a.text, "Break it");
        assert_eq!((a.button, a.icon), (true, Some(IconType::Hand)));
        let AimAction::Request(Request::Hit { kind, spawn, .. }) = a.action else {
            panic!("{:?}", a.action);
        };
        assert_eq!(kind, HitKind::Hand);
        for (s, e) in spawn.iter().zip([0.5, 2.0, 3.0]) {
            assert!((s - e).abs() < 1e-5, "{spawn:?}");
        }

        // The locker; nothing; a class the rules do not know.
        let a = aim(&t, &r, None, true, |_| true).unwrap();
        assert_eq!((a.text.as_str(), a.action), ("Open", AimAction::OpenLocker));
        assert_eq!((a.button, a.icon), (true, Some(IconType::Hand)));
        // `usePackUpIcon`: the pack-up text and icon.
        let mut r2 = rules();
        r2.pickups.get_mut("ti").unwrap().pack_up_icon = true;
        let a = aim(&t, &r2, Some(&ti), false, |_| true).unwrap();
        assert_eq!(
            (a.text.as_str(), a.icon),
            ("Pack up Ti", Some(IconType::PackUp))
        );
        assert!(aim(&t, &r, None, false, |_| true).is_none());
        let unknown = target("x", TargetKind::Pickup, placed);
        assert!(aim(&t, &r, Some(&unknown), false, |_| true).is_none());
    }
}
