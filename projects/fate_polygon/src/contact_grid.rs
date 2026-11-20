//! Spatial index for the lab's bounded, equal-radius disks.
//!
//! Each bucket is a bit set of stable body indices. A query reads neighboring
//! buckets in source-index order, matching the order of an all-pairs pass.

use fate_sim::math::{Fixed, FixedVec2};

use crate::settings::{MAX_BODIES, MAX_RADIUS_HUNDREDTHS};

const COLUMNS: usize = 21;
const ROWS: usize = 13;
const CELL_UNITS: i64 = 2;
const CELL_SIZE: Fixed = Fixed::from_int(CELL_UNITS);
// A colliding pair can span at most one cell along either axis.
const _: () = assert!(2 * (MAX_RADIUS_HUNDREDTHS as i64) < 100 * CELL_UNITS);
const WORDS: usize = (MAX_BODIES as usize).div_ceil(u64::BITS as usize);
type Bucket = [u64; WORDS];

#[derive(Default)]
pub struct ContactGrid {
    buckets: Vec<Bucket>,
    cells: Vec<usize>,
}

impl ContactGrid {
    pub fn rebuild(&mut self, positions: impl IntoIterator<Item = FixedVec2>) {
        self.buckets.resize(COLUMNS * ROWS, [0; WORDS]);
        self.buckets.fill([0; WORDS]);
        self.cells.clear();
        for (i, position) in positions.into_iter().enumerate() {
            let cell = cell(position);
            self.cells.push(cell);
            self.buckets[cell][i / 64] |= bit(i);
        }
    }

    /// Move a body and report whether its neighborhood may have changed.
    pub fn update(&mut self, i: usize, position: FixedVec2) -> bool {
        let next = cell(position);
        let previous = self.cells[i];
        if previous == next {
            return false;
        }
        self.buckets[previous][i / 64] &= !bit(i);
        self.buckets[next][i / 64] |= bit(i);
        self.cells[i] = next;
        true
    }

    pub fn preceding_neighbors(&self, i: usize, first: usize) -> Neighbors {
        let (x, y) = (self.cells[i] % COLUMNS, self.cells[i] / COLUMNS);
        let mut bits = [0; WORDS];
        for row in y.saturating_sub(1)..=(y + 1).min(ROWS - 1) {
            for col in x.saturating_sub(1)..=(x + 1).min(COLUMNS - 1) {
                for (dst, src) in bits.iter_mut().zip(self.buckets[row * COLUMNS + col]) {
                    *dst |= src;
                }
            }
        }
        let word = first / 64;
        if let Some(bits) = bits.get_mut(word) {
            *bits &= u64::MAX << (first % 64);
        }
        Neighbors {
            bits,
            word,
            stop: i,
        }
    }
}

pub struct Neighbors {
    bits: Bucket,
    word: usize,
    stop: usize,
}

impl Iterator for Neighbors {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        while let Some(word) = self.bits.get_mut(self.word) {
            if *word == 0 {
                self.word += 1;
                continue;
            }
            let i = self.word * 64 + word.trailing_zeros() as usize;
            if i >= self.stop {
                self.word = WORDS;
                return None;
            }
            *word &= *word - 1;
            return Some(i);
        }
        None
    }
}

fn cell(p: FixedVec2) -> usize {
    let index = |axis: Fixed, size: usize| {
        (axis.raw().div_euclid(CELL_SIZE.raw()) + (size / 2) as i64).clamp(0, size as i64 - 1)
            as usize
    };
    // Intermediate solver corrections can push a center outside the arena.
    // Clamping the cell index cannot separate two cells that were neighbors.
    index(p.y, ROWS) * COLUMNS + index(p.x, COLUMNS)
}

fn bit(i: usize) -> u64 {
    1 << (i % 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(cents: i64) -> FixedVec2 {
        FixedVec2::new(Fixed::from_ratio(cents, 100), Fixed::ZERO)
    }

    #[test]
    fn neighbors_are_sorted_across_words_and_updated_after_crossing_cells() {
        let mut grid = ContactGrid::default();
        grid.rebuild((0..=130).map(|i| {
            if [0, 63, 64, 129, 130].contains(&i) {
                at(-1)
            } else {
                at(1200)
            }
        }));
        assert_eq!(
            grid.preceding_neighbors(130, 64).collect::<Vec<_>>(),
            [64, 129]
        );
        assert!(grid.update(130, at(600)));
        assert!(grid.preceding_neighbors(130, 0).next().is_none());
        assert!(grid.update(64, at(799)));
        assert_eq!(grid.preceding_neighbors(130, 1).collect::<Vec<_>>(), [64]);
        assert!(grid.update(129, at(800))); // neighboring positive cell
        assert_eq!(grid.preceding_neighbors(130, 65).collect::<Vec<_>>(), [129]);
    }
}
