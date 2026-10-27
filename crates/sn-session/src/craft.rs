//! Replicated fabricator state. The server alone schedules completion.
use crate::items::{ItemRules, ItemsState};
use sn_protocol::codec::{Reader, Writer};
use sn_protocol::{ItemEvent, ItemId, PlayerId, RefuseReason};
use sn_sim::craft::{self, Pending};
use sn_sim::inventory::Item;

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub owner: PlayerId,
    pub start: u32,
    pub end: u32,
    pub done: bool,
    pub output: Pending,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fabricator {
    pub power: f32,
    pub job: Option<Job>,
}

impl Fabricator {
    pub(crate) fn encode(&self, w: &mut Writer) {
        w.f32(self.power);
        w.bool(self.job.is_some());
        if let Some(j) = &self.job {
            w.varint(u64::from(j.owner.0));
            w.varint(u64::from(j.start));
            w.varint(u64::from(j.end));
            w.bool(j.done);
            w.varint_signed(i64::from(j.output.recipe.tech));
            w.varint(u64::from(j.output.primary_left));
            w.varint(j.output.linked_index as u64);
        }
    }

    pub(crate) fn decode(r: &mut Reader, rules: &ItemRules) -> sn_protocol::Result<Self> {
        let at = r.pos;
        let power = r.f32("pod power")?;
        if power < 0.0 {
            return Err(r.error(at, "negative pod power"));
        }
        let job = if r.bool("craft job")? {
            let owner = PlayerId(r.varint_u16("craft owner")?);
            let start = r.varint_u32("craft start")?;
            let end = r.varint_u32("craft end")?;
            let done = r.bool("craft done")?;
            let tech = r.varint_i32("craft tech")?;
            let recipe = rules
                .recipes
                .get(&tech)
                .ok_or_else(|| r.error(at, "unknown craft recipe"))?
                .clone();
            let primary_left = r.varint_u32("craft amount")?;
            let linked_index = r.count(recipe.linked.len(), "craft linked index")?;
            if end <= start
                || primary_left > recipe.amount
                || (primary_left > 0 && linked_index != 0)
            {
                return Err(r.error(at, "invalid craft progress"));
            }
            Some(Job {
                owner,
                start,
                end,
                done,
                output: Pending {
                    recipe,
                    primary_left,
                    linked_index,
                },
            })
        } else {
            None
        };
        Ok(Self { power, job })
    }
}

fn reason(r: craft::Refused) -> RefuseReason {
    match r {
        craft::Refused::UnknownBlueprint => RefuseReason::UnknownBlueprint,
        craft::Refused::MissingIngredient(_) => RefuseReason::MissingIngredients,
        craft::Refused::NoPower => RefuseReason::NoPower,
        craft::Refused::InventoryFull => RefuseReason::NoRoom,
        craft::Refused::InvalidRecipe => RefuseReason::Unknown,
    }
}

impl ItemsState {
    pub(crate) fn apply_craft(
        &mut self,
        player: PlayerId,
        event: &ItemEvent,
    ) -> Result<(), RefuseReason> {
        let rules = self.rules().clone();
        match *event {
            ItemEvent::CraftStarted {
                crafter: 0,
                tech,
                start_tick,
                end_tick,
            } => {
                if self.fabricator.job.is_some() {
                    return Err(RefuseReason::Busy);
                }
                if end_tick <= start_tick {
                    return Err(RefuseReason::Unknown);
                }
                let r = rules.recipes.get(&tech).ok_or(RefuseReason::Unknown)?;
                let p = self.players.get_mut(&player).ok_or(RefuseReason::Unknown)?;
                let before: Vec<_> = p.inventory.container.items().map(|i| i.id).collect();
                let output = craft::consume(
                    r,
                    &p.inventory.known,
                    &mut p.inventory.container,
                    &mut self.fabricator.power,
                )
                .map_err(reason)?;
                for id in before {
                    if !p.inventory.container.contains(id) {
                        self.classes.remove(&ItemId(id.0 as u32));
                    }
                }
                self.fabricator.job = Some(Job {
                    owner: player,
                    start: start_tick,
                    end: end_tick,
                    done: false,
                    output,
                });
            }
            ItemEvent::CraftDone { crafter: 0 } => {
                let j = self.fabricator.job.as_mut().ok_or(RefuseReason::Unknown)?;
                if j.done {
                    return Err(RefuseReason::NotAllowed);
                }
                j.done = true;
            }
            ItemEvent::CraftedTaken { crafter: 0 } => {
                let mut job = self.fabricator.job.clone().ok_or(RefuseReason::Unknown)?;
                if !job.done {
                    return Err(RefuseReason::Busy);
                }
                let mut p = self
                    .players
                    .get(&player)
                    .ok_or(RefuseReason::Unknown)?
                    .clone();
                let mut next = self.next_item;
                let mut added = Vec::new();
                while let Some(tech) = job.output.current() {
                    let class = rules.tech_class.get(&tech).ok_or(RefuseReason::Unknown)?;
                    let info = rules.pickups.get(class).ok_or(RefuseReason::Unknown)?;
                    let id = next;
                    let after = next.checked_add(1).ok_or(RefuseReason::NotAllowed)?;
                    let item = Item {
                        id: sn_sim::inventory::ItemId(u64::from(id)),
                        tech,
                        destroy_on_death: info.destroy_on_death,
                        eatable: info.eatable,
                        charge: None,
                    };
                    match job
                        .output
                        .take_one(&rules.techs, &mut p.inventory.container, item)
                    {
                        Ok(()) => {}
                        Err(craft::Refused::InventoryFull) => break,
                        Err(e) => return Err(reason(e)),
                    }
                    p.inventory.known.analyze(&rules.analysis, tech);
                    added.push((ItemId(id), class.clone()));
                    next = after;
                }
                if added.is_empty() && job.output.current().is_some() {
                    return Err(RefuseReason::NoRoom);
                }
                self.players.insert(player, p);
                for (id, class) in added {
                    self.classes.insert(id, class);
                }
                self.next_item = next;
                self.fabricator.job = job.output.current().map(|_| job);
            }
            _ => return Err(RefuseReason::Unknown),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_sim::inventory::{Item, ItemId as SimId, PickupInfo};
    use std::sync::Arc;

    fn state() -> ItemsState {
        let mut rules = ItemRules {
            inventory_size: (2, 2),
            default_tech: vec![1001],
            pod_power: 10.0,
            ..Default::default()
        };
        rules.recipes.insert(
            1001,
            craft::Recipe {
                tech: 1001,
                ingredients: vec![(1002, 1)],
                amount: 2,
                linked: vec![1003],
                time: None,
            },
        );
        for tech in [1001, 1002, 1003] {
            let class = format!("synthetic-{tech}");
            rules.tech_class.insert(tech, class.clone());
            rules.pickups.insert(
                class,
                PickupInfo {
                    tech,
                    pickupable: true,
                    destroy_on_death: true,
                    eatable: false,
                    randomize_rotation: false,
                    pack_up_icon: false,
                },
            );
        }
        let mut s = ItemsState::new(Arc::new(rules), false);
        for player in [PlayerId(0), PlayerId(1)] {
            s.join(player);
        }
        let rules = s.rules().clone();
        s.players
            .get_mut(&PlayerId(0))
            .unwrap()
            .inventory
            .container
            .add(&rules.techs, Item::new(0, 1002))
            .unwrap();
        s.classes.insert(ItemId(0), "synthetic-1002".into());
        s.next_item = 1;
        s
    }

    #[test]
    fn shared_busy_state_snapshot_and_collection() {
        let mut s = state();
        let start = ItemEvent::CraftStarted {
            crafter: 0,
            tech: 1001,
            start_tick: 10,
            end_tick: 150,
        };
        s.apply(PlayerId(0), &start).unwrap();
        assert_eq!(s.fabricator.power, 5.0);
        assert!(!s.classes.contains_key(&ItemId(0)));
        assert_eq!(s.apply(PlayerId(1), &start), Err(RefuseReason::Busy));
        assert_eq!(
            s.apply(PlayerId(0), &ItemEvent::CraftedTaken { crafter: 0 }),
            Err(RefuseReason::Busy)
        );
        let bytes = s.to_bytes();
        let restored = ItemsState::decode(s.rules().clone(), &mut Reader::new(&bytes)).unwrap();
        assert_eq!(s, restored);
        s.apply(PlayerId(0), &ItemEvent::CraftDone { crafter: 0 })
            .unwrap();
        s.apply(PlayerId(1), &ItemEvent::CraftedTaken { crafter: 0 })
            .unwrap();
        let c = &s.players[&PlayerId(1)].inventory.container;
        assert_eq!((c.count_of(1001), c.count_of(1003)), (2, 1));
        assert!(s.fabricator.job.is_none());
        assert!(c.contains(SimId(1)));
        assert_eq!(s.next_item, 4);
    }

    #[test]
    fn refusal_and_malformed_snapshot_preserve_state() {
        let mut s = state();
        let before = s.clone();
        let event = ItemEvent::CraftStarted {
            crafter: 0,
            tech: 1001,
            start_tick: 10,
            end_tick: 150,
        };
        assert_eq!(
            s.apply(PlayerId(1), &event),
            Err(RefuseReason::MissingIngredients)
        );
        assert_eq!(s, before);
        s.apply(PlayerId(0), &event).unwrap();
        let bytes = s.to_bytes();
        for n in 0..bytes.len() {
            assert!(ItemsState::decode(s.rules().clone(), &mut Reader::new(&bytes[..n])).is_err());
        }
    }
}
