//! Fabricator rules, after reading GhostCrafter and CrafterLogic (M9e1).
//! Recipes and sizes are supplied by the caller, never embedded game data.

use crate::inventory::{Container, Item, KnownTech, TechType, Techs};

/// GhostCrafter.Craft's fixed cost, independent of TechData.energyCost.
pub const ENERGY: f32 = 5.0;
/// GhostCrafter's spawnAnimationDelay + spawnAnimationDuration.
pub const MIN_TIME: f32 = 1.0 + 1.7;
/// CrafterLogic.Craft adds this to its end time.
pub const END_DELAY: f32 = 0.1;

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub tech: TechType,
    pub ingredients: Vec<(TechType, u32)>,
    pub amount: u32,
    pub linked: Vec<TechType>,
    pub time: Option<f32>,
}

impl Recipe {
    pub fn animation_time(&self) -> f32 {
        self.time.unwrap_or(MIN_TIME).max(MIN_TIME)
    }

    pub fn duration(&self) -> f32 {
        self.animation_time() + END_DELAY
    }
}

/// Survival-mode checks. The UI checks unlocks and ingredients before
/// invoking GhostCrafter; room is checked only when collecting each item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    UnknownBlueprint,
    MissingIngredient(TechType),
    NoPower,
    InventoryFull,
    InvalidRecipe,
}

pub fn can_craft(
    recipe: &Recipe,
    known: &KnownTech,
    grid: &Container,
    power: f32,
) -> Result<(), Refused> {
    if recipe.tech == 0 || recipe.time.is_some_and(|t| !t.is_finite() || t < 0.0) {
        return Err(Refused::InvalidRecipe);
    }
    if !known.knows(recipe.tech) {
        return Err(Refused::UnknownBlueprint);
    }
    for &(tech, amount) in &recipe.ingredients {
        if grid.count_of(tech) < amount as usize {
            return Err(Refused::MissingIngredient(tech));
        }
    }
    if !power.is_finite() || power < ENERGY {
        return Err(Refused::NoPower);
    }
    Ok(())
}

/// Validate before mutating; remove from the inventory grid only, in recipe
/// order, using ItemsContainer's last/emptiest-battery selection.
pub fn consume(
    recipe: &Recipe,
    known: &KnownTech,
    grid: &mut Container,
    power: &mut f32,
) -> Result<Pending, Refused> {
    can_craft(recipe, known, grid, *power)?;
    // Reject duplicate ingredient rows whose combined amount is unavailable.
    // Shipped recipes have distinct rows; this protects caller-supplied data.
    let mut trial = grid.clone();
    for &(tech, amount) in &recipe.ingredients {
        for _ in 0..amount {
            trial
                .remove_tech(tech)
                .ok_or(Refused::MissingIngredient(tech))?;
        }
    }
    *grid = trial;
    *power -= ENERGY;
    Ok(Pending {
        recipe: recipe.clone(),
        primary_left: recipe.amount,
        linked_index: 0,
    })
}

/// Output remains in the machine if collecting it runs out of room. Primary
/// items precede linked items; each linked entry produces exactly one item.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub recipe: Recipe,
    pub primary_left: u32,
    pub linked_index: usize,
}

impl Pending {
    pub fn current(&self) -> Option<TechType> {
        if self.primary_left > 0 {
            Some(self.recipe.tech)
        } else {
            self.recipe.linked.get(self.linked_index).copied()
        }
    }

    /// The caller supplies the id and prefab-derived flags of the next item.
    /// Time eligibility belongs to the caller's world clock.
    pub fn take_one(
        &mut self,
        techs: &Techs,
        grid: &mut Container,
        item: Item,
    ) -> Result<(), Refused> {
        if self.current() != Some(item.tech) {
            return Err(Refused::InvalidRecipe);
        }
        grid.add(techs, item).map_err(|_| Refused::InventoryFull)?;
        if self.primary_left > 0 {
            self.primary_left -= 1;
        } else {
            self.linked_index += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Recipe {
        Recipe {
            tech: 1001,
            ingredients: vec![(1002, 1)],
            amount: 2,
            linked: vec![1003, 1003],
            time: None,
        }
    }
    fn known() -> KnownTech {
        KnownTech::new(&[], &[1001])
    }

    #[test]
    fn checks_and_consumes_only_after_all_checks_pass() {
        let r = recipe();
        let t = Techs::default();
        let mut c = Container::new(2, 2);
        let mut power = 10.0;
        assert_eq!(
            can_craft(&r, &KnownTech::default(), &c, power),
            Err(Refused::UnknownBlueprint)
        );
        assert_eq!(
            consume(&r, &known(), &mut c, &mut power),
            Err(Refused::MissingIngredient(1002))
        );
        assert_eq!(power, 10.0);
        c.add(&t, Item::new(1, 1002)).unwrap();
        assert_eq!(can_craft(&r, &known(), &c, 4.99), Err(Refused::NoPower));
        consume(&r, &known(), &mut c, &mut power).unwrap();
        assert!(c.is_empty());
        assert_eq!(power, 5.0);
    }

    #[test]
    fn partial_collection_retains_primary_and_linked_outputs() {
        let r = recipe();
        let t = Techs::default();
        let mut c = Container::new(1, 1);
        c.add(&t, Item::new(1, 1002)).unwrap();
        let mut p = consume(&r, &known(), &mut c, &mut 5.0).unwrap();
        for (id, tech) in [(2, 1001), (3, 1001), (4, 1003), (5, 1003)] {
            p.take_one(&t, &mut c, Item::new(id, tech)).unwrap();
            let before = p.clone();
            if let Some(next) = p.current() {
                assert_eq!(
                    p.take_one(&t, &mut c, Item::new(99, next)),
                    Err(Refused::InventoryFull)
                );
                assert_eq!(p, before);
            }
            c.remove(crate::inventory::ItemId(id));
        }
        assert_eq!(p.current(), None);
    }

    #[test]
    fn full_inventory_does_not_prevent_starting_a_craft() {
        let r = Recipe {
            ingredients: vec![],
            ..recipe()
        };
        let mut c = Container::new(1, 1);
        c.add(&Techs::default(), Item::new(1, 1004)).unwrap();
        assert!(consume(&r, &known(), &mut c, &mut 5.0).is_ok());
    }

    #[test]
    fn timing_and_zero_primary_linked_recipe() {
        let mut r = recipe();
        assert_eq!(r.animation_time(), 2.7);
        assert_eq!(r.duration(), 2.7 + 0.1);
        r.time = Some(1.0);
        assert_eq!(r.animation_time(), 2.7);
        r.time = Some(8.0);
        assert_eq!(r.duration(), 8.1);
        r.amount = 0;
        r.ingredients.clear();
        let p = consume(&r, &known(), &mut Container::new(1, 1), &mut 5.0).unwrap();
        assert_eq!(p.current(), Some(1003));
    }

    #[test]
    fn malformed_recipe_does_not_partially_consume() {
        let mut r = recipe();
        let mut c = Container::new(1, 1);
        c.add(&Techs::default(), Item::new(1, 1002)).unwrap();
        r.ingredients.push((1002, 1));
        let mut power = 10.0;
        assert!(consume(&r, &known(), &mut c, &mut power).is_err());
        assert_eq!(c.len(), 1);
        assert_eq!(power, 10.0);
        r.time = Some(f32::NAN);
        assert_eq!(
            can_craft(&r, &known(), &c, power),
            Err(Refused::InvalidRecipe)
        );
    }
}
