//! `KnownTech`: the blueprints a player knows, and `Analyze`, which
//! unlocks the ones `PDAData.analysisTech` lists for a tech type the
//! first time it is analysed (on every item added to the inventory).
//! Compound tech, scanner entries and story goals are not ported yet.

use std::collections::BTreeSet;

use super::{TECH_NONE, TechType};

/// One `PDAData.analysisTech` entry.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisEntry {
    pub tech: TechType,
    pub unlocks: Vec<TechType>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnownTech {
    known: BTreeSet<TechType>,
    analyzed: BTreeSet<TechType>,
}

impl KnownTech {
    /// `Initialize`: knows `PDAData.defaultTech` (`AddRange`, which
    /// analyses each).
    pub fn new(table: &[AnalysisEntry], default: &[TechType]) -> KnownTech {
        let mut k = KnownTech::default();
        let mut out = Vec::new();
        for &tech in default {
            if k.known.insert(tech) {
                k.analyze_into(table, tech, &mut out);
            }
        }
        k
    }

    pub fn knows(&self, tech: TechType) -> bool {
        self.known.contains(&tech)
    }

    pub fn known(&self) -> impl Iterator<Item = TechType> + '_ {
        self.known.iter().copied()
    }

    pub fn analyzed(&self, tech: TechType) -> bool {
        self.analyzed.contains(&tech)
    }

    /// `Analyze`: the tech types newly known, in the order the game adds
    /// them.
    pub fn analyze(&mut self, table: &[AnalysisEntry], tech: TechType) -> Vec<TechType> {
        let mut out = Vec::new();
        self.analyze_into(table, tech, &mut out);
        out
    }

    /// `Add`: known now, and analysed in turn.
    pub fn add(&mut self, table: &[AnalysisEntry], tech: TechType) -> Vec<TechType> {
        let mut out = Vec::new();
        self.add_into(table, tech, &mut out);
        out
    }

    fn analyze_into(&mut self, table: &[AnalysisEntry], tech: TechType, out: &mut Vec<TechType>) {
        if tech == TECH_NONE {
            return;
        }
        // The first entry for the type wins: after it, the type is in
        // `analyzedTech` and the others are skipped.
        if let Some(entry) = table.iter().find(|e| e.tech == tech) {
            if self.analyzed.insert(tech) {
                for &u in &entry.unlocks {
                    self.add_into(table, u, out);
                }
            }
        }
    }

    fn add_into(&mut self, table: &[AnalysisEntry], tech: TechType, out: &mut Vec<TechType>) {
        if tech != TECH_NONE && self.known.insert(tech) {
            out.push(tech);
            // Each type is analysed once, so the depth is bounded by the
            // table.
            self.analyze_into(table, tech, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<AnalysisEntry> {
        let e = |tech, unlocks: &[TechType]| AnalysisEntry {
            tech,
            unlocks: unlocks.to_vec(),
        };
        vec![e(10, &[20, 21]), e(20, &[30]), e(10, &[99]), e(30, &[10])]
    }

    #[test]
    fn analysing_unlocks_once_and_chains() {
        let t = table();
        let mut k = KnownTech::default();
        // 10 → 20 (→ 30 → 10 already analysed), 21. The second entry for
        // 10 is never used.
        assert_eq!(k.analyze(&t, 10), [20, 30, 10, 21]);
        assert!(k.knows(30) && !k.knows(99));
        assert_eq!(k.analyze(&t, 10), []);
        assert_eq!(k.analyze(&t, 5), []);
        assert!(!k.analyzed(5));
    }

    #[test]
    fn default_tech_is_known_and_analysed() {
        let k = KnownTech::new(&table(), &[20]);
        assert_eq!(k.known().collect::<Vec<_>>(), [10, 20, 21, 30]);
    }
}
