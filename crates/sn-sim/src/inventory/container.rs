//! `ItemsContainer`: a grid of `width` × `height` cells holding items
//! grouped by tech type. Positions are not stored: every change re-packs
//! the whole grid (`TrySort`), so they are a function of the contents and
//! their order, computed by [`Container::layout`].

use std::cmp::Ordering;

use super::{DictOrder, Item, ItemId, TECH_NONE, TechType, Techs};

/// All items of one tech type (`ItemsContainer.ItemGroup`), in the order
/// they were added.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    tech: TechType,
    width: i32,
    height: i32,
    items: Vec<Item>,
}

/// An item's cells after packing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: ItemId,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Our bound on a side, far above the sizes read so far (6 × 8 and
/// 4 × 8): a bad size cannot make the grid huge.
pub const MAX_SIDE: i32 = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    width: i32,
    height: i32,
    /// `_items`, a `Dictionary<TechType, ItemGroup>`.
    groups: DictOrder<Group>,
    count: usize,
}

/// A group as the packer sees it: the ghost of `HasRoomFor` has no items,
/// only a count of one.
struct Packing<'a> {
    id: TechType,
    width: i32,
    height: i32,
    ghost: bool,
    items: &'a [Item],
}

impl Container {
    /// `new ItemsContainer(width, height, …)`.
    pub fn new(width: i32, height: i32) -> Container {
        Container {
            width: width.clamp(0, MAX_SIDE),
            height: height.clamp(0, MAX_SIDE),
            groups: DictOrder::default(),
            count: 0,
        }
    }

    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The items in the game's enumeration order: groups in their
    /// dictionary order, each group's items in the order they came.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.groups.iter().flat_map(|g| g.items.iter())
    }

    pub fn get(&self, id: ItemId) -> Option<&Item> {
        self.items().find(|i| i.id == id)
    }

    pub fn contains(&self, id: ItemId) -> bool {
        self.get(id).is_some()
    }

    /// How many items of `tech` (`GetCount`).
    pub fn count_of(&self, tech: TechType) -> usize {
        self.groups
            .iter()
            .find(|g| g.tech == tech)
            .map_or(0, |g| g.items.len())
    }

    /// `HasRoomFor(TechType)`.
    pub fn has_room_for(&self, techs: &Techs, tech: TechType) -> bool {
        let (w, h) = techs.size(tech);
        self.has_room_for_size(w, h, None)
    }

    /// `HasRoomFor(width, height)`, with the item `ignore` left out of
    /// the packing (`IItemsContainer.HasRoomFor(pickupable, ignore)`, used
    /// by a swap).
    pub fn has_room_for_size(&self, width: i32, height: i32, ignore: Option<ItemId>) -> bool {
        self.pack(Some((width, height)), ignore, None)
    }

    /// `AddItem` for an item that is in no container: refused (and given
    /// back) without room.
    pub fn add(&mut self, techs: &Techs, item: Item) -> Result<(), Item> {
        if !self.has_room_for(techs, item.tech) {
            return Err(item);
        }
        self.unsafe_add(techs, item);
        Ok(())
    }

    /// `UnsafeAdd`: adds without checking for room.
    pub fn unsafe_add(&mut self, techs: &Techs, item: Item) {
        let tech = item.tech;
        match self.groups.position(|g| g.tech == tech) {
            Some(i) => {
                if let Some(g) = self.groups.get_mut(i) {
                    g.items.push(item);
                }
            }
            None => {
                let (width, height) = techs.size(tech);
                self.groups.insert(Group {
                    tech,
                    width,
                    height,
                    items: vec![item],
                });
            }
        }
        self.count += 1;
    }

    /// `RemoveItem(Pickupable)`.
    pub fn remove(&mut self, id: ItemId) -> Option<Item> {
        let gi = self
            .groups
            .position(|g| g.items.iter().any(|i| i.id == id))?;
        let g = self.groups.get_mut(gi)?;
        let ii = g.items.iter().position(|i| i.id == id)?;
        let item = g.items.remove(ii);
        if g.items.is_empty() {
            self.groups.remove(gi);
        }
        self.count -= 1;
        Some(item)
    }

    /// `RemoveItem(TechType)`: the last item of the type, except that
    /// among the batteries at the end of the group the one with the
    /// least charge goes (the latest of equals).
    pub fn remove_tech(&mut self, tech: TechType) -> Option<Item> {
        let gi = self.groups.position(|g| g.tech == tech)?;
        let g = self.groups.get_mut(gi)?;
        let mut pick = g.items.len().checked_sub(1)?;
        let mut least = f32::MAX;
        for (i, item) in g.items.iter().enumerate().rev() {
            let Some(charge) = item.charge else { break };
            if least > charge {
                least = charge;
                pick = i;
            }
        }
        let id = g.items.get(pick)?.id;
        self.remove(id)
    }

    /// Where every item sits: `Sort` (`TrySort` over all groups). Items
    /// that did not fit (only after an `unsafe_add` past the room) are
    /// missing, and the flag is false.
    pub fn layout(&self) -> (Vec<Placed>, bool) {
        let mut out = Vec::with_capacity(self.count);
        let ok = self.pack(None, None, Some(&mut out));
        (out, ok)
    }

    /// `TrySort`: groups in `GroupComparer`'s order, each item in the
    /// first free cell (`GetRoomFor`).
    fn pack(
        &self,
        ghost: Option<(i32, i32)>,
        ignore: Option<ItemId>,
        mut out: Option<&mut Vec<Placed>>,
    ) -> bool {
        let mut groups: Vec<Packing> = self
            .groups
            .iter()
            .map(|g| Packing {
                id: g.tech,
                width: g.width,
                height: g.height,
                ghost: false,
                items: &g.items,
            })
            .collect();
        if let Some((width, height)) = ghost {
            // The ghost group's id is 0 (`TechType.None`).
            groups.push(Packing {
                id: TECH_NONE,
                width,
                height,
                ghost: true,
                items: &[],
            });
        }
        groups.sort_by(group_order);
        let mut map = Map::new(self.width, self.height);
        for g in &groups {
            let n = if g.ghost { 1 } else { g.items.len() };
            for k in 0..n {
                let item = g.items.get(k);
                if item.is_some_and(|i| Some(i.id) == ignore) {
                    continue;
                }
                let Some((x, y)) = map.room_for(g.width, g.height) else {
                    return false;
                };
                map.fill(x, y, g.width, g.height);
                if let (Some(out), Some(item)) = (out.as_deref_mut(), item) {
                    out.push(Placed {
                        id: item.id,
                        x,
                        y,
                        width: g.width,
                        height: g.height,
                    });
                }
            }
        }
        true
    }
}

/// `GroupComparer.Compare`: larger side first, then larger area, at equal
/// area the taller first, then the lower tech type number. The game's
/// comparer calls two groups with the same id equal; that happens only
/// for the ghost against items of `TechType.None`, where we put the
/// ghost last.
fn group_order(a: &Packing, b: &Packing) -> Ordering {
    if a.id == b.id {
        return a.ghost.cmp(&b.ghost);
    }
    let side_a = a.width.max(a.height);
    let side_b = b.width.max(b.height);
    if side_a != side_b {
        return side_b.cmp(&side_a);
    }
    let area_a = i64::from(a.width) * i64::from(a.height);
    let area_b = i64::from(b.width) * i64::from(b.height);
    if area_a == area_b {
        if a.height != b.height {
            return b.height.cmp(&a.height);
        }
    } else {
        return area_b.cmp(&area_a);
    }
    a.id.cmp(&b.id)
}

/// `itemsMap`: per cell, the column after the item covering it (what
/// `NextFreeCell` reads: `item.x + item.width`).
struct Map {
    width: i32,
    height: i32,
    cells: Vec<Option<i32>>,
}

impl Map {
    fn new(width: i32, height: i32) -> Map {
        let n = usize::try_from(i64::from(width) * i64::from(height)).unwrap_or(0);
        Map {
            width,
            height,
            cells: vec![None; n],
        }
    }

    fn cell(&self, x: i32, y: i32) -> Option<i32> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return None;
        }
        let i = usize::try_from(i64::from(y) * i64::from(self.width) + i64::from(x)).ok()?;
        self.cells.get(i).copied().flatten()
    }

    /// `NextFreeCell`: `x` if the `w` × `h` box at (`x`, `y`) is free,
    /// else the column after the first item found in it (rows top down,
    /// columns left to right).
    fn next_free(&self, x: i32, y: i32, w: i32, h: i32) -> i32 {
        for yy in y..y.saturating_add(h) {
            for xx in x..x.saturating_add(w) {
                if let Some(end) = self.cell(xx, yy) {
                    return end;
                }
            }
        }
        x
    }

    /// `GetRoomFor`: rows top to bottom, in each the leftmost column that
    /// fits.
    fn room_for(&self, w: i32, h: i32) -> Option<(i32, i32)> {
        let mut y = 0;
        while y <= self.height.saturating_sub(h) {
            let mut x = 0;
            while x <= self.width.saturating_sub(w) {
                let next = self.next_free(x, y, w, h);
                if next == x {
                    return Some((x, y));
                }
                // An item's end is always right of the cell it covers.
                x = next.max(x + 1);
            }
            y += 1;
        }
        None
    }

    /// `AddItemToMap`.
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let end = x.saturating_add(w);
        for yy in y..y.saturating_add(h) {
            for xx in x..end {
                if xx < 0 || yy < 0 || xx >= self.width || yy >= self.height {
                    continue;
                }
                let i = (yy * self.width + xx) as usize;
                if let Some(c) = self.cells.get_mut(i) {
                    *c = Some(end);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1: 1×1, 2: 2×2, 3: 3×3, 4: 1×2 (tall), 5: 2×1 (wide).
    fn techs() -> Techs {
        let mut t = Techs::default();
        t.sizes.insert(2, (2, 2));
        t.sizes.insert(3, (3, 3));
        t.sizes.insert(4, (1, 2));
        t.sizes.insert(5, (2, 1));
        t
    }

    fn cells(c: &Container) -> Vec<(u64, i32, i32)> {
        let (placed, ok) = c.layout();
        assert!(ok);
        placed.iter().map(|p| (p.id.0, p.x, p.y)).collect()
    }

    /// The grid drawn as rows of tech digits ('.' free).
    fn picture(c: &Container) -> Vec<String> {
        let (w, h) = c.size();
        let mut rows = vec![vec!['.'; w as usize]; h as usize];
        for p in c.layout().0 {
            let tech = c.get(p.id).unwrap().tech;
            for y in p.y..p.y + p.height {
                for x in p.x..p.x + p.width {
                    rows[y as usize][x as usize] = char::from(b'0' + tech as u8);
                }
            }
        }
        rows.into_iter().map(String::from_iter).collect()
    }

    #[test]
    fn mixed_sizes_pack_bigger_first_then_fill_the_gaps() {
        let t = techs();
        let mut c = Container::new(6, 8);
        let mut id = 0;
        // Added small first: the order of adding does not matter.
        for tech in [1, 1, 1, 4, 5, 2, 3, 1] {
            id += 1;
            c.add(&t, Item::new(id, tech)).unwrap();
        }
        assert_eq!(
            picture(&c),
            [
                "333224", // 3×3, 2×2, then the tall 1×2 in the next gap
                "333224", "333551", // the wide 2×1 (equal area: the taller went first)
                "111...", // then the 1×1s, in the order they came
                "......", "......", "......", "......",
            ]
        );
    }

    #[test]
    fn hand_worked_rows() {
        let t = techs();
        let mut c = Container::new(6, 8);
        // Four 2×2 and four 1×1: 2×2s at x 0, 2, 4 in row 0, then (0, 2).
        for i in 0..4 {
            c.add(&t, Item::new(10 + i, 2)).unwrap();
        }
        for i in 0..4 {
            c.add(&t, Item::new(20 + i, 1)).unwrap();
        }
        assert_eq!(
            cells(&c),
            [
                (10, 0, 0),
                (11, 2, 0),
                (12, 4, 0),
                (13, 0, 2),
                (20, 2, 2),
                (21, 3, 2),
                (22, 4, 2),
                (23, 5, 2),
            ]
        );
    }

    #[test]
    fn a_full_grid_refuses() {
        let t = techs();
        let mut c = Container::new(6, 8);
        for i in 0..48 {
            c.add(&t, Item::new(i, 1)).unwrap();
        }
        assert!(!c.has_room_for(&t, 1));
        assert_eq!(c.add(&t, Item::new(99, 1)).unwrap_err().id, ItemId(99));
        assert_eq!(c.len(), 48);
    }

    #[test]
    fn room_depends_on_the_shape_left() {
        let t = techs();
        let mut c = Container::new(6, 8);
        // 45 cells of 1×1 taken, 3 free: room for a 2×1, not a 2×2, and
        // wherever the free cells are, since every check re-packs.
        for i in 0..45 {
            c.add(&t, Item::new(i, 1)).unwrap();
        }
        assert!(c.has_room_for(&t, 5));
        assert!(!c.has_room_for(&t, 2));
        // Re-packing can make room: two 2×2 and 1×1 that would not fit
        // in arrival order, fit sorted.
        let mut c = Container::new(4, 4);
        for i in 0..8 {
            c.add(&t, Item::new(i, 1)).unwrap();
        }
        assert!(c.has_room_for(&t, 2));
        c.add(&t, Item::new(50, 2)).unwrap();
        assert!(c.has_room_for(&t, 2));
        c.add(&t, Item::new(51, 2)).unwrap();
        assert_eq!(picture(&c), ["2222", "2222", "1111", "1111"]);
    }

    #[test]
    fn removal_repacks() {
        let t = techs();
        let mut c = Container::new(6, 8);
        c.add(&t, Item::new(1, 1)).unwrap();
        c.add(&t, Item::new(2, 2)).unwrap();
        c.add(&t, Item::new(3, 1)).unwrap();
        assert_eq!(cells(&c), [(2, 0, 0), (1, 2, 0), (3, 3, 0)]);
        assert_eq!(c.remove(ItemId(2)).unwrap().tech, 2);
        assert_eq!(cells(&c), [(1, 0, 0), (3, 1, 0)]);
        assert_eq!(c.remove(ItemId(2)), None);
    }

    #[test]
    fn ignoring_an_item_frees_its_cells_for_the_trial() {
        let t = techs();
        let mut c = Container::new(2, 2);
        c.add(&t, Item::new(1, 2)).unwrap();
        assert!(!c.has_room_for_size(1, 1, None));
        assert!(c.has_room_for_size(2, 2, Some(ItemId(1))));
    }

    #[test]
    fn remove_by_tech_takes_the_last_or_the_emptiest_battery() {
        let t = techs();
        let mut c = Container::new(6, 8);
        for i in 0..3 {
            c.add(&t, Item::new(i, 1)).unwrap();
        }
        assert_eq!(c.remove_tech(1).unwrap().id, ItemId(2));
        let mut c = Container::new(6, 8);
        for (i, charge) in [
            (0, Some(5.0)),
            (1, None),
            (2, Some(3.0)),
            (3, Some(9.0)),
            (4, Some(3.0)),
        ] {
            let mut item = Item::new(i, 7);
            item.charge = charge;
            c.add(&t, item).unwrap();
        }
        // From the end: 3.0 at 4, 9.0, 3.0 at 2 (not less), then a
        // non-battery stops the search.
        assert_eq!(c.remove_tech(7).unwrap().id, ItemId(4));
        assert_eq!(c.remove_tech(7).unwrap().id, ItemId(2));
        assert_eq!(c.remove_tech(42), None);
    }

    #[test]
    fn group_order_follows_the_game_comparer() {
        let p = |id, width, height| Packing {
            id,
            width,
            height,
            ghost: false,
            items: &[],
        };
        // Larger side first, even with a smaller area.
        assert_eq!(group_order(&p(1, 3, 1), &p(2, 2, 2)), Ordering::Less);
        // Same side, larger area first.
        assert_eq!(group_order(&p(1, 2, 1), &p(2, 2, 2)), Ordering::Greater);
        // Same area, taller first.
        assert_eq!(group_order(&p(1, 2, 1), &p(2, 1, 2)), Ordering::Greater);
        // Same shape: lower id first.
        assert_eq!(group_order(&p(9, 1, 1), &p(2, 1, 1)), Ordering::Greater);
    }

    #[test]
    fn odd_sizes_never_panic() {
        let mut t = Techs::default();
        t.sizes.insert(1, (0, 0));
        t.sizes.insert(2, (-3, 2));
        t.sizes.insert(3, (7, 1));
        t.sizes.insert(4, (i32::MAX, i32::MAX));
        for (w, h) in [(0, 0), (-1, 5), (6, 8), (1, 1)] {
            let mut c = Container::new(w, h);
            for tech in [1, 2, 3, 4, 5] {
                let _ = c.add(&t, Item::new(tech as u64, tech));
            }
            let _ = c.layout();
        }
    }
}
