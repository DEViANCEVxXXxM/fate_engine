use crate::math::Fixed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub const fn state(self) -> u64 {
        self.state
    }

    pub const fn from_state(state: u64) -> Self {
        Self { state }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.state)
    }

    pub fn range_u32(&mut self, upper_exclusive: u32) -> u32 {
        self.range_u64(upper_exclusive as u64) as u32
    }

    pub fn range_u64(&mut self, upper_exclusive: u64) -> u64 {
        assert!(upper_exclusive > 0, "random range must be non-empty");
        let threshold = upper_exclusive.wrapping_neg() % upper_exclusive;
        loop {
            let value = self.next_u64();
            let product = value as u128 * upper_exclusive as u128;
            let low = product as u64;
            if low >= threshold {
                return (product >> 64) as u64;
            }
        }
    }

    pub fn range_i32(&mut self, min_inclusive: i32, max_exclusive: i32) -> i32 {
        assert!(
            min_inclusive < max_exclusive,
            "random range must be non-empty"
        );
        let range = (max_exclusive as i64 - min_inclusive as i64) as u64;
        (min_inclusive as i64 + self.range_u64(range) as i64) as i32
    }

    pub fn range_i64(&mut self, min_inclusive: i64, max_exclusive: i64) -> i64 {
        assert!(
            min_inclusive < max_exclusive,
            "random range must be non-empty"
        );
        let range = (max_exclusive as i128 - min_inclusive as i128) as u64;
        (min_inclusive as i128 + self.range_u64(range) as i128) as i64
    }

    pub fn range_fixed(&mut self, min_inclusive: Fixed, max_exclusive: Fixed) -> Fixed {
        Fixed::from_raw(self.range_i64(min_inclusive.raw(), max_exclusive.raw()))
    }

    pub fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            let other = self.range_u64((index + 1) as u64) as usize;
            values.swap(index, other);
        }
    }
}

fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_stream_matches_golden_values() {
        let mut rng = DeterministicRng::new(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn generator_state_restores_the_exact_future_sequence() {
        let mut first = DeterministicRng::new(0x1234_5678);
        for _ in 0..17 {
            first.next_u64();
        }
        let mut restored = DeterministicRng::from_state(first.state());

        for _ in 0..100 {
            assert_eq!(first.next_u64(), restored.next_u64());
        }
    }

    #[test]
    fn bounded_values_stay_inside_the_requested_range() {
        let mut rng = DeterministicRng::new(42);
        for upper in [1, 2, 3, 10, u32::MAX] {
            for _ in 0..1000 {
                assert!(rng.range_u32(upper) < upper);
            }
        }
    }

    #[test]
    fn signed_ranges_cover_negative_values_and_full_i32_span() {
        let mut rng = DeterministicRng::new(0xface);
        for _ in 0..10_000 {
            let signed = rng.range_i32(-17, 23);
            assert!((-17..23).contains(&signed));

            let full_span = rng.range_i32(i32::MIN, i32::MAX);
            assert!((i32::MIN..i32::MAX).contains(&full_span));
        }
    }

    #[test]
    fn signed_i64_and_fixed_ranges_are_bounded_and_reproducible() {
        let mut first = DeterministicRng::new(0xface);
        let mut replay = DeterministicRng::new(0xface);
        for _ in 0..1000 {
            let integer = first.range_i64(i64::MIN, i64::MAX);
            assert!((i64::MIN..i64::MAX).contains(&integer));
            assert_eq!(integer, replay.range_i64(i64::MIN, i64::MAX));

            let fixed = first.range_fixed(Fixed::from_raw(-1000), Fixed::from_raw(1000));
            assert!((-1000..1000).contains(&fixed.raw()));
            assert_eq!(
                fixed,
                replay.range_fixed(Fixed::from_raw(-1000), Fixed::from_raw(1000))
            );
        }
    }

    #[test]
    fn seeded_shuffle_is_reproducible_and_preserves_every_element() {
        let mut first: Vec<_> = (0..32).collect();
        let mut second = first.clone();
        let mut different = first.clone();

        DeterministicRng::new(123).shuffle(&mut first);
        DeterministicRng::new(123).shuffle(&mut second);
        DeterministicRng::new(456).shuffle(&mut different);

        assert_eq!(first, second);
        assert_ne!(first, different);
        first.sort_unstable();
        assert_eq!(first, (0..32).collect::<Vec<_>>());
    }

    #[test]
    fn empty_and_single_element_shuffles_are_no_ops() {
        let mut rng = DeterministicRng::new(5);
        let mut empty: [u8; 0] = [];
        let mut one = [42];

        rng.shuffle(&mut empty);
        rng.shuffle(&mut one);

        assert_eq!(one, [42]);
    }
}
