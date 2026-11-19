use super::{Bounds2d, Texture2d, TextureRegion};
use crate::math::Vec2;
use std::ops::Range;

/// Maximum number of visible tiles uploaded in one tilemap frame.
pub const MAX_DRAWN_TILES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileMapError {
    InvalidDimensions,
    InvalidGeometry,
    WrongCellCount { expected: usize, actual: usize },
}

impl std::fmt::Display for TileMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDimensions => {
                f.write_str("tilemap dimensions must be nonzero and fit in memory")
            }
            Self::InvalidGeometry => {
                f.write_str("tilemap needs finite world coordinates and positive tile dimensions")
            }
            Self::WrongCellCount { expected, actual } => {
                write!(f, "tilemap expects {expected} cells, got {actual}")
            }
        }
    }
}

impl std::error::Error for TileMapError {}

/// CPU-owned row-major grid. Row zero begins at `top_left`; columns go
/// right (+X), and rows go down (-Y). A `None` cell is empty; a tile ID
/// indexes the region palette of the layer that draws this map.
pub struct TileMap2d {
    size: (u32, u32),
    top_left: Vec2,
    tile_size: Vec2,
    cells: Vec<Option<u16>>,
}

impl TileMap2d {
    pub fn new(
        size: (u32, u32),
        top_left: Vec2,
        tile_size: Vec2,
        cells: Vec<Option<u16>>,
    ) -> Result<Self, TileMapError> {
        let expected = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .filter(|&len| len != 0 && len <= isize::MAX as usize / size_of::<Option<u16>>())
            .ok_or(TileMapError::InvalidDimensions)?;
        let (right, bottom) = (
            f64::from(top_left.x) + f64::from(tile_size.x) * f64::from(size.0),
            f64::from(top_left.y) - f64::from(tile_size.y) * f64::from(size.1),
        );
        if !top_left.x.is_finite()
            || !top_left.y.is_finite()
            || !tile_size.x.is_finite()
            || !tile_size.y.is_finite()
            || tile_size.x <= 0.0
            || tile_size.y <= 0.0
            || !(right as f32).is_finite()
            || !(bottom as f32).is_finite()
        {
            return Err(TileMapError::InvalidGeometry);
        }
        if cells.len() != expected {
            return Err(TileMapError::WrongCellCount {
                expected,
                actual: cells.len(),
            });
        }
        Ok(Self {
            size,
            top_left,
            tile_size,
            cells,
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        self.size
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub fn tile_size(&self) -> Vec2 {
        self.tile_size
    }

    fn index(&self, (column, row): (u32, u32)) -> Option<usize> {
        (column < self.size.0 && row < self.size.1)
            .then(|| row as usize * self.size.0 as usize + column as usize)
    }

    /// Distinguishes out of bounds (`None`) from an empty cell (`Some(None)`).
    pub fn get(&self, cell: (u32, u32)) -> Option<Option<u16>> {
        self.index(cell).map(|index| self.cells[index])
    }

    /// Replace one cell. Returns false and makes no change if out of bounds.
    pub fn set(&mut self, cell: (u32, u32), tile: Option<u16>) -> bool {
        let Some(index) = self.index(cell) else {
            return false;
        };
        self.cells[index] = tile;
        true
    }

    /// Convert a point into (column, row), including empty cells. Intervals
    /// are half-open: the right and bottom edges of the map are out of bounds.
    pub fn cell_at_world(&self, world: Vec2) -> Option<(u32, u32)> {
        let x = self.column(f64::from(world.x)).floor();
        let y = self.row(f64::from(world.y)).floor();
        if x >= 0.0 && x < f64::from(self.size.0) && y >= 0.0 && y < f64::from(self.size.1) {
            Some((x as u32, y as u32))
        } else {
            None
        }
    }

    fn column(&self, x: f64) -> f64 {
        (x - f64::from(self.top_left.x)) / f64::from(self.tile_size.x)
    }

    fn row(&self, y: f64) -> f64 {
        (f64::from(self.top_left.y) - y) / f64::from(self.tile_size.y)
    }

    pub(super) fn world_edges(&self, (column, row): (u32, u32)) -> [f32; 4] {
        let x = |index| (f64::from(self.top_left.x) + f64::from(self.tile_size.x) * index) as f32;
        let y = |index| (f64::from(self.top_left.y) - f64::from(self.tile_size.y) * index) as f32;
        [
            x(f64::from(column)),
            y(f64::from(row)),
            x(f64::from(column) + 1.0),
            y(f64::from(row) + 1.0),
        ]
    }

    fn visible_ranges(&self, view: Bounds2d) -> (Range<u32>, Range<u32>) {
        let x = f64::from(view.center.x);
        let y = f64::from(view.center.y);
        let half_x = f64::from(view.half_extent.x);
        let half_y = f64::from(view.half_extent.y);
        if !(x.is_finite() && y.is_finite() && half_x.is_finite() && half_y.is_finite())
            || half_x <= 0.0
            || half_y <= 0.0
        {
            return (0..0, 0..0);
        }
        let axis = |min: f64, max: f64, count| {
            min.floor().clamp(0.0, f64::from(count)) as u32
                ..max.ceil().clamp(0.0, f64::from(count)) as u32
        };
        (
            axis(
                self.column(x - half_x),
                self.column(x + half_x),
                self.size.0,
            ),
            axis(self.row(y + half_y), self.row(y - half_y), self.size.1),
        )
    }

    /// Iterate occupied cells overlapping a world-space view, in row-major
    /// order. This visits only the intersecting row and column ranges.
    pub fn visible_cells(&self, view: Bounds2d) -> impl Iterator<Item = ((u32, u32), u16)> + '_ {
        let (columns, mut rows) = self.visible_ranges(view);
        if columns.is_empty() {
            rows = 0..0;
        }
        rows.flat_map(move |row| {
            columns.clone().filter_map(move |column| {
                let id = self.cells[row as usize * self.size.0 as usize + column as usize]?;
                Some(((column, row), id))
            })
        })
    }
}

/// Draw one orthogonal tile grid using a single atlas. Non-finite regions
/// and tile IDs outside the palette are skipped.
#[derive(Clone, Copy)]
pub struct TileLayer2d<'a> {
    pub map: &'a TileMap2d,
    pub texture: &'a Texture2d,
    /// Region table indexed by tile ID; regions may reuse or flip atlas tiles.
    pub regions: &'a [TextureRegion],
}

impl<'a> TileLayer2d<'a> {
    pub const fn new(
        map: &'a TileMap2d,
        texture: &'a Texture2d,
        regions: &'a [TextureRegion],
    ) -> Self {
        Self {
            map,
            texture,
            regions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TileMap2d {
        TileMap2d::new(
            (3, 2),
            Vec2::new(10.0, 20.0),
            Vec2::new(2.0, 3.0),
            vec![Some(0), None, Some(2), Some(3), Some(4), Some(5)],
        )
        .unwrap()
    }

    #[test]
    fn coordinates_and_edits_respect_half_open_grid_edges() {
        let mut map = sample();
        assert_eq!(map.cell_at_world(Vec2::new(10.0, 20.0)), Some((0, 0)));
        assert_eq!(map.cell_at_world(Vec2::new(12.0, 17.0)), Some((1, 1)));
        assert_eq!(map.cell_at_world(Vec2::new(16.0, 16.0)), None);
        assert_eq!(map.cell_at_world(Vec2::new(12.0, 14.0)), None);
        assert_eq!(map.cell_at_world(Vec2::new(f32::NAN, 19.0)), None);
        assert_eq!(map.world_edges((1, 1)), [12.0, 17.0, 14.0, 14.0]);
        assert_eq!(map.get((1, 0)), Some(None));
        assert!(map.set((1, 0), Some(8)));
        assert_eq!(map.get((1, 0)), Some(Some(8)));
        assert!(!map.set((3, 0), Some(8)));
        assert_eq!(map.get((3, 0)), None);
    }

    #[test]
    fn visibility_visits_only_overlapping_cells_in_row_major_order() {
        let map = sample();
        let view = |x, y, w, h| Bounds2d::from_center_size(Vec2::new(x, y), Vec2::new(w, h));
        assert_eq!(
            map.visible_cells(view(12.0, 17.0, 4.0, 6.0))
                .collect::<Vec<_>>(),
            [((0, 0), 0), ((0, 1), 3), ((1, 1), 4)]
        );
        assert_eq!(
            map.visible_cells(view(16.5, 17.5, 2.0, 2.0))
                .collect::<Vec<_>>(),
            [((2, 0), 2), ((2, 1), 5)]
        );
        assert_eq!(map.visible_cells(view(17.0, 19.0, 2.0, 2.0)).count(), 0);
        assert_eq!(map.visible_cells(view(12.0, 17.0, 0.0, 4.0)).count(), 0);
    }

    #[test]
    fn invalid_geometry_and_cell_counts_are_rejected() {
        assert!(matches!(
            TileMap2d::new((0, 1), Vec2::ZERO, Vec2::ONE, vec![]),
            Err(TileMapError::InvalidDimensions)
        ));
        assert!(matches!(
            TileMap2d::new((2, 1), Vec2::ZERO, Vec2::ONE, vec![]),
            Err(TileMapError::WrongCellCount {
                expected: 2,
                actual: 0
            })
        ));
        assert!(matches!(
            TileMap2d::new((2, 1), Vec2::ZERO, Vec2::new(f32::MAX, 1.0), vec![None; 2]),
            Err(TileMapError::InvalidGeometry)
        ));
    }
}
