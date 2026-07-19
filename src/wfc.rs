use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use crate::{CellId, Direction, Topology};

/// A stable, dense identifier for one pattern in a [`WfcRules`] table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PatternId(usize);

impl PatternId {
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn index(self) -> usize {
        self.0
    }
}

/// An error constructing WFC rules.
#[derive(Clone, Debug, PartialEq)]
pub enum WfcError {
    NoPatterns,
    InvalidWeight { pattern: PatternId, weight: f64 },
    RuleTableOverflow { patterns: usize, directions: usize },
}

impl fmt::Display for WfcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPatterns => write!(f, "WFC needs at least one pattern"),
            Self::InvalidWeight { pattern, weight } => write!(
                f,
                "WFC pattern {} has invalid weight {weight}; weights must be finite and positive",
                pattern.index()
            ),
            Self::RuleTableOverflow {
                patterns,
                directions,
            } => write!(
                f,
                "WFC compatibility table overflows usize for {patterns} patterns and {directions} directions"
            ),
        }
    }
}

impl Error for WfcError {}

/// Immutable weighted, directional compatibility rules for WFC patterns.
///
/// `compatible(direction, source, neighbor)` is evaluated for every ordered
/// topology neighbor relationship. Callers can therefore model asymmetric
/// constraints when needed.
#[derive(Clone, Debug, PartialEq)]
pub struct WfcRules<D> {
    weights: Vec<f64>,
    weight_log_weights: Vec<f64>,
    compatible: Vec<bool>,
    direction: PhantomData<fn(D) -> D>,
}

impl<D: Direction> WfcRules<D> {
    pub fn new(
        weights: impl IntoIterator<Item = f64>,
        mut compatible: impl FnMut(D, PatternId, PatternId) -> bool,
    ) -> Result<Self, WfcError> {
        let weights: Vec<_> = weights.into_iter().collect();
        if weights.is_empty() {
            return Err(WfcError::NoPatterns);
        }
        for (index, weight) in weights.iter().copied().enumerate() {
            if !weight.is_finite() || weight <= 0.0 {
                return Err(WfcError::InvalidWeight {
                    pattern: PatternId::new(index),
                    weight,
                });
            }
        }

        let pair_count = weights
            .len()
            .checked_mul(weights.len())
            .and_then(|pairs| pairs.checked_mul(D::ALL.len()))
            .ok_or(WfcError::RuleTableOverflow {
                patterns: weights.len(),
                directions: D::ALL.len(),
            })?;
        let mut compatibility = Vec::with_capacity(pair_count);
        for direction in D::ALL.iter().copied() {
            for source in 0..weights.len() {
                for neighbor in 0..weights.len() {
                    compatibility.push(compatible(
                        direction,
                        PatternId::new(source),
                        PatternId::new(neighbor),
                    ));
                }
            }
        }

        let weight_log_weights = weights.iter().map(|weight| weight * weight.ln()).collect();
        Ok(Self {
            weights,
            weight_log_weights,
            compatible: compatibility,
            direction: PhantomData,
        })
    }

    pub fn pattern_count(&self) -> usize {
        self.weights.len()
    }

    pub fn weight(&self, pattern: PatternId) -> Option<f64> {
        self.weights.get(pattern.index()).copied()
    }

    pub fn allows(&self, direction: D, source: PatternId, neighbor: PatternId) -> bool {
        let patterns = self.pattern_count();
        let Some(direction_offset) = direction.index().checked_mul(patterns * patterns) else {
            return false;
        };
        let Some(source_offset) = source.index().checked_mul(patterns) else {
            return false;
        };
        let Some(index) = direction_offset
            .checked_add(source_offset)
            .and_then(|index| index.checked_add(neighbor.index()))
        else {
            return false;
        };
        self.compatible.get(index).copied().unwrap_or(false)
    }
}

/// Current terminal or non-terminal state of a WFC wave.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WfcStatus {
    Running,
    Solved,
    Contradiction { cell: CellId },
}

/// One weighted observation and the propagation it triggered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WfcStep {
    pub cell: CellId,
    pub pattern: PatternId,
    pub removed_candidates: usize,
    pub status: WfcStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Domain {
    allowed: Vec<bool>,
    len: usize,
}

impl Domain {
    fn from_fn(pattern_count: usize, mut allowed: impl FnMut(PatternId) -> bool) -> Self {
        let allowed: Vec<_> = (0..pattern_count)
            .map(|index| allowed(PatternId::new(index)))
            .collect();
        let len = allowed.iter().filter(|candidate| **candidate).count();
        Self { allowed, len }
    }

    fn contains(&self, pattern: PatternId) -> bool {
        self.allowed.get(pattern.index()).copied().unwrap_or(false)
    }

    fn retain(&mut self, mut keep: impl FnMut(PatternId) -> bool) -> usize {
        let before = self.len;
        for (index, allowed) in self.allowed.iter_mut().enumerate() {
            if *allowed && !keep(PatternId::new(index)) {
                *allowed = false;
                self.len -= 1;
            }
        }
        before - self.len
    }
}

/// A seeded Wave Function Collapse state over an arbitrary finite [`Topology`].
///
/// The solver owns only dense pattern domains and compatibility rules. Pattern
/// payloads, tile rendering, and other application data remain with the caller.
#[derive(Clone, Debug)]
pub struct Wfc<T: Topology> {
    topology: T,
    rules: WfcRules<T::Direction>,
    initial_domains: Vec<Domain>,
    domains: Vec<Domain>,
    status: WfcStatus,
    seed: u64,
    random: SplitMix64,
}

impl<T: Topology> Wfc<T> {
    /// Starts a wave with every pattern initially allowed in every cell.
    pub fn new(topology: T, rules: WfcRules<T::Direction>, seed: u64) -> Self {
        Self::with_constraints(topology, rules, seed, |_cell, _pattern| true)
    }

    /// Starts a wave with application-defined initial candidate constraints.
    ///
    /// The predicate is stored as dense domains. [`Self::restart`] restores
    /// exactly these constraints before propagating them again.
    pub fn with_constraints(
        topology: T,
        rules: WfcRules<T::Direction>,
        seed: u64,
        mut initially_allowed: impl FnMut(CellId, PatternId) -> bool,
    ) -> Self {
        let initial_domains: Vec<Domain> = (0..topology.cell_count())
            .map(|index| {
                let cell = CellId::new(index);
                Domain::from_fn(rules.pattern_count(), |pattern| {
                    initially_allowed(cell, pattern)
                })
            })
            .collect();
        let mut wave = Self {
            topology,
            rules,
            domains: initial_domains.clone(),
            initial_domains,
            status: WfcStatus::Running,
            seed,
            random: SplitMix64::new(seed),
        };
        wave.propagate_all();
        wave
    }

    pub const fn status(&self) -> WfcStatus {
        self.status
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub fn topology(&self) -> &T {
        &self.topology
    }

    pub fn rules(&self) -> &WfcRules<T::Direction> {
        &self.rules
    }

    pub fn candidate_count(&self, cell: CellId) -> Option<usize> {
        self.domains.get(cell.index()).map(|domain| domain.len)
    }

    pub fn candidates(&self, cell: CellId) -> Option<impl Iterator<Item = PatternId> + '_> {
        let domain = self.domains.get(cell.index())?;
        Some(
            domain
                .allowed
                .iter()
                .enumerate()
                .filter_map(|(index, allowed)| allowed.then_some(PatternId::new(index))),
        )
    }

    pub fn collapsed_pattern(&self, cell: CellId) -> Option<PatternId> {
        let domain = self.domains.get(cell.index())?;
        if domain.len != 1 {
            return None;
        }
        domain
            .allowed
            .iter()
            .position(|allowed| *allowed)
            .map(PatternId::new)
    }

    /// Returns Shannon entropy for a non-empty cell domain.
    pub fn entropy(&self, cell: CellId) -> Option<f64> {
        let domain = self.domains.get(cell.index())?;
        self.domain_entropy(domain)
    }

    /// Restores the initial constraints and changes the deterministic random seed.
    pub fn restart(&mut self, seed: u64) -> WfcStatus {
        self.seed = seed;
        self.random = SplitMix64::new(seed);
        self.domains.clone_from(&self.initial_domains);
        self.status = WfcStatus::Running;
        self.propagate_all();
        self.status
    }

    /// Observes the lowest-entropy cell once, then propagates to a fixed point.
    pub fn step(&mut self) -> Option<WfcStep> {
        if self.status != WfcStatus::Running {
            return None;
        }

        let mut best_entropy = f64::INFINITY;
        let mut best_cells = Vec::new();
        for index in 0..self.domains.len() {
            let domain = &self.domains[index];
            if domain.len <= 1 {
                continue;
            }
            let entropy = self
                .domain_entropy(domain)
                .expect("a multi-pattern domain has entropy");
            if entropy + f64::EPSILON < best_entropy {
                best_entropy = entropy;
                best_cells.clear();
                best_cells.push(CellId::new(index));
            } else if (entropy - best_entropy).abs() <= f64::EPSILON {
                best_cells.push(CellId::new(index));
            }
        }

        if best_cells.is_empty() {
            self.update_status();
            return None;
        }
        let cell = best_cells[self.random.index(best_cells.len())];
        let pattern = self.choose_pattern(cell);
        let removed_by_observation =
            self.domains[cell.index()].retain(|candidate| candidate == pattern);
        let removed_by_propagation = self.propagate_from([cell]);
        Some(WfcStep {
            cell,
            pattern,
            removed_candidates: removed_by_observation + removed_by_propagation,
            status: self.status,
        })
    }

    fn domain_entropy(&self, domain: &Domain) -> Option<f64> {
        if domain.len == 0 {
            return None;
        }
        let mut weight_sum = 0.0;
        let mut weight_log_weight_sum = 0.0;
        for (index, allowed) in domain.allowed.iter().copied().enumerate() {
            if allowed {
                weight_sum += self.rules.weights[index];
                weight_log_weight_sum += self.rules.weight_log_weights[index];
            }
        }
        Some(weight_sum.ln() - weight_log_weight_sum / weight_sum)
    }

    fn choose_pattern(&mut self, cell: CellId) -> PatternId {
        let domain = &self.domains[cell.index()];
        let total_weight: f64 = domain
            .allowed
            .iter()
            .enumerate()
            .filter(|(_, allowed)| **allowed)
            .map(|(index, _)| self.rules.weights[index])
            .sum();
        let mut choice = self.random.unit_f64() * total_weight;
        let mut fallback = None;
        for (index, allowed) in domain.allowed.iter().copied().enumerate() {
            if !allowed {
                continue;
            }
            let pattern = PatternId::new(index);
            fallback = Some(pattern);
            let weight = self.rules.weights[index];
            if choice < weight {
                return pattern;
            }
            choice -= weight;
        }
        fallback.expect("a running wave observes a non-empty domain")
    }

    fn propagate_all(&mut self) -> usize {
        self.propagate_from((0..self.domains.len()).map(CellId::new))
    }

    fn propagate_from(&mut self, cells: impl IntoIterator<Item = CellId>) -> usize {
        let mut queue: VecDeque<_> = cells.into_iter().collect();
        let mut removed = 0;

        while let Some(source) = queue.pop_front() {
            if self.domains[source.index()].len == 0 {
                self.status = WfcStatus::Contradiction { cell: source };
                return removed;
            }
            for direction in T::Direction::ALL.iter().copied() {
                let Some(neighbor) = self.topology.neighbor(source, direction) else {
                    continue;
                };
                let mut unsupported = Vec::new();
                for neighbor_index in 0..self.rules.pattern_count() {
                    let neighbor_pattern = PatternId::new(neighbor_index);
                    if !self.domains[neighbor.index()].contains(neighbor_pattern) {
                        continue;
                    }
                    let supported = if source == neighbor {
                        self.rules
                            .allows(direction, neighbor_pattern, neighbor_pattern)
                    } else {
                        (0..self.rules.pattern_count()).any(|source_index| {
                            let source_pattern = PatternId::new(source_index);
                            self.domains[source.index()].contains(source_pattern)
                                && self
                                    .rules
                                    .allows(direction, source_pattern, neighbor_pattern)
                        })
                    };
                    if !supported {
                        unsupported.push(neighbor_pattern);
                    }
                }

                if unsupported.is_empty() {
                    continue;
                }
                let domain = &mut self.domains[neighbor.index()];
                let removed_here = domain.retain(|pattern| !unsupported.contains(&pattern));
                removed += removed_here;
                if domain.len == 0 {
                    self.status = WfcStatus::Contradiction { cell: neighbor };
                    return removed;
                }
                queue.push_back(neighbor);
            }
        }

        self.update_status();
        removed
    }

    fn update_status(&mut self) {
        if let Some(index) = self.domains.iter().position(|domain| domain.len == 0) {
            self.status = WfcStatus::Contradiction {
                cell: CellId::new(index),
            };
        } else if self.domains.iter().all(|domain| domain.len == 1) {
            self.status = WfcStatus::Solved;
        } else {
            self.status = WfcStatus::Running;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn index(&mut self, upper: usize) -> usize {
        debug_assert!(upper > 0);
        ((self.next_u64() as u128 * upper as u128) >> 64) as usize
    }

    fn unit_f64(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / ((1_u64 << 53) as f64);
        (self.next_u64() >> 11) as f64 * SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Coord2, Extent2, SquareDirection, SquareTopology};

    fn all_compatible_rules(patterns: usize) -> WfcRules<SquareDirection> {
        WfcRules::new(vec![1.0; patterns], |_direction, _source, _neighbor| true).unwrap()
    }

    fn equality_rules(patterns: usize) -> WfcRules<SquareDirection> {
        WfcRules::new(vec![1.0; patterns], |_direction, source, neighbor| {
            source == neighbor
        })
        .unwrap()
    }

    #[test]
    fn rules_validate_patterns_weights_and_table_lookups() {
        assert_eq!(
            WfcRules::<SquareDirection>::new([], |_direction, _source, _neighbor| true),
            Err(WfcError::NoPatterns)
        );
        assert!(matches!(
            WfcRules::<SquareDirection>::new([0.0], |_direction, _source, _neighbor| true),
            Err(WfcError::InvalidWeight { .. })
        ));

        let rules = WfcRules::new([1.0, 2.0], |direction, source, neighbor| {
            direction == SquareDirection::East
                && source == PatternId::new(0)
                && neighbor == PatternId::new(1)
        })
        .unwrap();
        assert_eq!(rules.pattern_count(), 2);
        assert_eq!(rules.weight(PatternId::new(1)), Some(2.0));
        assert!(rules.allows(SquareDirection::East, PatternId::new(0), PatternId::new(1)));
        assert!(!rules.allows(SquareDirection::West, PatternId::new(0), PatternId::new(1)));
    }

    #[test]
    fn initial_constraints_propagate_to_a_complete_solution() {
        let topology = SquareTopology::bounded(Extent2::new(3, 1)).unwrap();
        let left = topology.cell_at(Coord2::ZERO).unwrap();
        let wave = Wfc::with_constraints(topology, equality_rules(2), 7, |cell, pattern| {
            cell != left || pattern == PatternId::new(1)
        });

        assert_eq!(wave.status(), WfcStatus::Solved);
        for index in 0..3 {
            assert_eq!(
                wave.collapsed_pattern(CellId::new(index)),
                Some(PatternId::new(1))
            );
        }
    }

    #[test]
    fn incompatible_initial_constraints_report_a_contradiction() {
        let topology = SquareTopology::bounded(Extent2::new(2, 1)).unwrap();
        let wave = Wfc::with_constraints(topology, equality_rules(2), 0, |cell, pattern| {
            pattern.index() == cell.index()
        });

        assert!(matches!(wave.status(), WfcStatus::Contradiction { .. }));
    }

    #[test]
    fn observation_uses_the_lowest_entropy_cell() {
        let topology = SquareTopology::bounded(Extent2::new(2, 1)).unwrap();
        let wave_rules = all_compatible_rules(3);
        let mut wave = Wfc::with_constraints(topology, wave_rules, 5, |cell, pattern| {
            cell.index() == 1 || pattern.index() < 2
        });

        let step = wave.step().unwrap();
        assert_eq!(step.cell, CellId::new(0));
        assert_eq!(wave.candidate_count(CellId::new(0)), Some(1));
    }

    #[test]
    fn weighted_observation_prefers_an_overwhelming_weight() {
        let topology = SquareTopology::bounded(Extent2::new(1, 1)).unwrap();
        let rules = WfcRules::new(
            [f64::MIN_POSITIVE, 1.0],
            |_direction, _source, _neighbor| true,
        )
        .unwrap();
        let mut wave = Wfc::new(topology, rules, 0);

        assert_eq!(wave.step().unwrap().pattern, PatternId::new(1));
    }

    #[test]
    fn equal_seeds_produce_equal_complete_waves() {
        let solve = |seed| {
            let topology = SquareTopology::bounded(Extent2::new(5, 3)).unwrap();
            let mut wave = Wfc::new(topology, all_compatible_rules(4), seed);
            while wave.step().is_some() {}
            (0..15)
                .map(|index| wave.collapsed_pattern(CellId::new(index)).unwrap())
                .collect::<Vec<_>>()
        };

        assert_eq!(solve(42), solve(42));
        assert_ne!(solve(42), solve(43));
    }

    #[test]
    fn restart_restores_constraints_and_uses_the_new_seed() {
        let topology = SquareTopology::bounded(Extent2::new(3, 1)).unwrap();
        let mut wave =
            Wfc::with_constraints(topology, all_compatible_rules(3), 1, |cell, pattern| {
                cell.index() != 0 || pattern == PatternId::new(2)
            });
        wave.step();

        assert_eq!(wave.restart(9), WfcStatus::Running);
        assert_eq!(wave.seed(), 9);
        assert_eq!(
            wave.collapsed_pattern(CellId::new(0)),
            Some(PatternId::new(2))
        );
        assert_eq!(wave.candidate_count(CellId::new(1)), Some(3));
    }

    #[test]
    fn directional_rules_propagate_in_their_topology_direction() {
        let topology = SquareTopology::bounded(Extent2::new(2, 1)).unwrap();
        let rules = WfcRules::new([1.0, 1.0], |direction, source, neighbor| match direction {
            SquareDirection::East => source.index() == 0 && neighbor.index() == 1,
            SquareDirection::West => source.index() == 1 && neighbor.index() == 0,
            SquareDirection::North | SquareDirection::South => true,
        })
        .unwrap();
        let wave = Wfc::new(topology, rules, 0);

        assert_eq!(wave.status(), WfcStatus::Solved);
        assert_eq!(
            wave.collapsed_pattern(CellId::new(0)),
            Some(PatternId::new(0))
        );
        assert_eq!(
            wave.collapsed_pattern(CellId::new(1)),
            Some(PatternId::new(1))
        );
    }

    #[test]
    fn empty_topology_is_already_solved() {
        let topology = SquareTopology::bounded(Extent2::new(0, 4)).unwrap();
        let mut wave = Wfc::new(topology, all_compatible_rules(2), 0);
        assert_eq!(wave.status(), WfcStatus::Solved);
        assert_eq!(wave.step(), None);
    }

    #[test]
    fn self_neighbors_require_each_candidate_to_support_itself() {
        let topology = SquareTopology::toroidal(Extent2::new(1, 1)).unwrap();
        let rules = WfcRules::new([1.0, 1.0], |_direction, source, neighbor| {
            source == neighbor && source == PatternId::new(0)
        })
        .unwrap();
        let wave = Wfc::new(topology, rules, 0);

        assert_eq!(wave.status(), WfcStatus::Solved);
        assert_eq!(
            wave.collapsed_pattern(CellId::new(0)),
            Some(PatternId::new(0))
        );
    }
}
