//! Deterministic unit-cost routing for Polygon's lab agents.

use fate_sim::math::{Fixed as F, FixedVec2 as V};

const WIDTH: usize = 32;
const HEIGHT: usize = 20;
const CELLS: usize = WIDTH * HEIGHT;
const UNREACHED: u16 = u16::MAX;

#[derive(Clone, Copy)]
pub struct Wall {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

pub const WALLS: [Wall; 6] = [
    Wall {
        x: 7,
        y: 0,
        width: 1,
        height: 13,
    },
    Wall {
        x: 7,
        y: 17,
        width: 1,
        height: 3,
    },
    Wall {
        x: 15,
        y: 0,
        width: 1,
        height: 4,
    },
    Wall {
        x: 15,
        y: 8,
        width: 1,
        height: 12,
    },
    Wall {
        x: 23,
        y: 0,
        width: 1,
        height: 12,
    },
    Wall {
        x: 23,
        y: 16,
        width: 1,
        height: 4,
    },
];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RouteWork {
    pub searches: u64,
    pub visits: u64,
    pub moves: u64,
}

pub struct Navigation {
    open: [bool; CELLS],
    scratch: Search,
    shared: Search,
    shared_target: Option<usize>,
}

impl Default for Navigation {
    fn default() -> Self {
        let mut open = [true; CELLS];
        for wall in WALLS {
            for y in wall.y..wall.y + wall.height {
                for x in wall.x..wall.x + wall.width {
                    open[y * WIDTH + x] = false;
                }
            }
        }
        Self {
            open,
            scratch: Search::default(),
            shared: Search::default(),
            shared_target: None,
        }
    }
}

const fn cell(x: usize, y: usize) -> usize {
    y * WIDTH + x
}

pub fn target(tick: u64, period: u32) -> usize {
    const GOALS: [usize; 4] = [cell(29, 17), cell(2, 17), cell(2, 2), cell(29, 2)];
    GOALS[((tick / u64::from(period)) % GOALS.len() as u64) as usize]
}

impl Wall {
    pub fn center(self) -> V {
        let center =
            |start, size, axis: usize| F::from_ratio((2 * start + size) as i64 - axis as i64, 2);
        V::new(
            center(self.x, self.width, WIDTH),
            center(self.y, self.height, HEIGHT),
        )
    }
}

pub fn position(cell: usize) -> V {
    let center = |coord, axis: usize| F::from_ratio((2 * coord + 1) as i64 - axis as i64, 2);
    V::new(center(cell % WIDTH, WIDTH), center(cell / WIDTH, HEIGHT))
}

impl Navigation {
    pub fn start_cell(&self, p: V) -> usize {
        let axis = |p: F, size: usize| {
            (p.raw().div_euclid(F::ONE.raw()) + (size / 2) as i64).clamp(0, size as i64 - 1)
                as usize
        };
        let (x, y) = (axis(p.x, WIDTH), axis(p.y, HEIGHT));
        let index = cell(x, y);
        if self.open[index] {
            return index;
        }
        // Only needed on a mode switch/spawn if the position falls in a wall.
        self.open
            .iter()
            .enumerate()
            .filter(|&(_, &free)| free)
            .min_by_key(|&(i, _)| (x.abs_diff(i % WIDTH) + y.abs_diff(i / WIDTH), i))
            .expect("open navigation map")
            .0
    }

    pub fn route(
        &mut self,
        start: usize,
        target: usize,
        shared: bool,
        work: &mut RouteWork,
    ) -> usize {
        if start == target {
            return start;
        }
        let field = if shared {
            if self.shared_target != Some(target) {
                self.shared.build(&self.open, target, None, work);
                self.shared_target = Some(target);
            }
            &self.shared
        } else {
            self.scratch.build(&self.open, target, Some(start), work);
            &self.scratch
        };
        let cost = field.distance[start];
        // A reverse BFS that stops at start has already visited every
        // shorter distance, so both modes choose the same first neighbor.
        let next = neighbors(start)
            .find(|&adjacent| field.distance[adjacent].checked_add(1) == Some(cost))
            .unwrap_or(start);
        work.moves += u64::from(start != next);
        next
    }
}

struct Search {
    distance: [u16; CELLS],
    queue: Vec<usize>,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            distance: [UNREACHED; CELLS],
            queue: Vec::new(),
        }
    }
}

impl Search {
    fn build(
        &mut self,
        open: &[bool; CELLS],
        target: usize,
        stop: Option<usize>,
        work: &mut RouteWork,
    ) {
        self.distance.fill(UNREACHED);
        self.queue.clear();
        self.queue.push(target);
        self.distance[target] = 0;
        work.searches += 1;
        let mut head = 0;
        while let Some(&current) = self.queue.get(head) {
            head += 1;
            work.visits += 1;
            if stop == Some(current) {
                break;
            }
            let next_cost = self.distance[current] + 1;
            for next in neighbors(current) {
                if open[next] && self.distance[next] == UNREACHED {
                    self.distance[next] = next_cost;
                    self.queue.push(next);
                }
            }
        }
    }
}

fn neighbors(index: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (index % WIDTH, index / WIDTH);
    [
        (x > 0).then(|| index - 1),
        (x + 1 < WIDTH).then_some(index + 1),
        (y > 0).then(|| index - WIDTH),
        (y + 1 < HEIGHT).then_some(index + WIDTH),
    ]
    .into_iter()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_search_and_shared_field_route_around_walls_in_the_same_order() {
        let mut nav = Navigation::default();
        let mut per_agent = RouteWork::default();
        let mut shared = RouteWork::default();
        let start = cell(6, 2);
        let target = cell(9, 2);
        let mut current = start;
        while current != target {
            let next = nav.route(current, target, false, &mut per_agent);
            assert_eq!(next, nav.route(current, target, true, &mut shared));
            assert!(nav.open[next] && next != current);
            current = next;
            assert!(per_agent.moves < 64);
        }
        assert_eq!(shared.searches, 1);
        assert_eq!(shared.moves, 25);
        assert_eq!(per_agent.moves, shared.moves);
        nav.route(target, start, true, &mut shared);
        assert_eq!(shared.searches, 2);
    }

    #[test]
    fn start_and_target_coordinates_snap_without_crossing_bounds() {
        let nav = Navigation::default();
        for i in 0..CELLS {
            let snapped = nav.start_cell(position(i));
            if nav.open[i] {
                assert_eq!(snapped, i);
            } else {
                assert!(nav.open[snapped]);
            }
        }
        for period in [1, 240] {
            for i in 0..4 {
                assert!(nav.open[target(i * u64::from(period), period)]);
            }
        }
    }
}
