//! Lazy, inclusive integer-key ranges for a single integer primary key.

/// The source key representation. Signedness is preserved at every boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerKey {
    Signed(i64),
    Unsigned(u64),
}

/// One closed range: `lower <= key <= upper`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntegerRange {
    pub lower: IntegerKey,
    pub upper: IntegerKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeError {
    ZeroSpan,
    IncompleteBounds,
    MixedSignedness,
    ReversedBounds,
}

/// Lazily partitions a key-space interval into ranges of at most `max_span`
/// adjacent integer values. It does not estimate or promise rows per range.
#[derive(Clone, Debug)]
pub struct IntegerRanges {
    next: Option<u128>,
    maximum: u128,
    max_span: u128,
    signed: bool,
}

impl IntegerRanges {
    /// `None, None` represents an empty table; one missing endpoint is invalid.
    /// Every produced range uses inclusive bounds, so the final endpoint needs
    /// no representable successor (including when it is `u64::MAX`).
    pub fn new(
        minimum: Option<IntegerKey>,
        maximum: Option<IntegerKey>,
        max_span: u64,
    ) -> Result<Self, RangeError> {
        if max_span == 0 {
            return Err(RangeError::ZeroSpan);
        }

        let (Some(minimum), Some(maximum)) = (minimum, maximum) else {
            return if minimum.is_none() && maximum.is_none() {
                Ok(Self {
                    next: None,
                    maximum: 0,
                    max_span: u128::from(max_span),
                    signed: false,
                })
            } else {
                Err(RangeError::IncompleteBounds)
            };
        };

        let (minimum_signed, minimum_coordinate) = coordinate(minimum);
        let (maximum_signed, maximum_coordinate) = coordinate(maximum);
        if minimum_signed != maximum_signed {
            return Err(RangeError::MixedSignedness);
        }
        if minimum_coordinate > maximum_coordinate {
            return Err(RangeError::ReversedBounds);
        }

        Ok(Self {
            next: Some(minimum_coordinate),
            maximum: maximum_coordinate,
            max_span: u128::from(max_span),
            signed: minimum_signed,
        })
    }
}

impl Iterator for IntegerRanges {
    type Item = IntegerRange;

    fn next(&mut self) -> Option<Self::Item> {
        let lower = self.next?;
        let upper = lower
            .checked_add(self.max_span - 1)
            .expect("u64 key coordinates and span fit in u128")
            .min(self.maximum);
        self.next = if upper == self.maximum {
            None
        } else {
            Some(
                upper
                    .checked_add(1)
                    .expect("a non-final u64 key coordinate has a successor"),
            )
        };

        Some(IntegerRange {
            lower: key_from_coordinate(lower, self.signed),
            upper: key_from_coordinate(upper, self.signed),
        })
    }
}

fn coordinate(key: IntegerKey) -> (bool, u128) {
    match key {
        IntegerKey::Signed(value) => (
            true,
            u128::try_from(i128::from(value) - i128::from(i64::MIN))
                .expect("signed offset is non-negative and fits in u64"),
        ),
        IntegerKey::Unsigned(value) => (false, u128::from(value)),
    }
}

fn key_from_coordinate(coordinate: u128, signed: bool) -> IntegerKey {
    if signed {
        IntegerKey::Signed(
            i64::try_from(
                i128::from(i64::MIN)
                    + i128::try_from(coordinate).expect("key coordinate fits within u64"),
            )
            .expect("signed coordinate originated within the i64 domain"),
        )
    } else {
        IntegerKey::Unsigned(
            u64::try_from(coordinate)
                .expect("unsigned coordinate originated within the u64 domain"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{IntegerKey as Key, IntegerRange, IntegerRanges, RangeError};

    fn ranges(minimum: Key, maximum: Key, span: u64) -> Vec<IntegerRange> {
        IntegerRanges::new(Some(minimum), Some(maximum), span)
            .unwrap()
            .collect()
    }

    #[test]
    fn empty_and_single_value_intervals() {
        assert_eq!(IntegerRanges::new(None, None, 5).unwrap().next(), None);
        assert_eq!(
            ranges(Key::Signed(-7), Key::Signed(-7), 1),
            [IntegerRange {
                lower: Key::Signed(-7),
                upper: Key::Signed(-7),
            }]
        );
    }

    #[test]
    fn signed_ranges_cross_zero_and_keep_an_uneven_final_range() {
        assert_eq!(
            ranges(Key::Signed(-5), Key::Signed(3), 4),
            [
                IntegerRange {
                    lower: Key::Signed(-5),
                    upper: Key::Signed(-2),
                },
                IntegerRange {
                    lower: Key::Signed(-1),
                    upper: Key::Signed(2),
                },
                IntegerRange {
                    lower: Key::Signed(3),
                    upper: Key::Signed(3),
                },
            ]
        );
    }

    #[test]
    fn full_signed_domain_uses_wide_coordinates() {
        assert_eq!(
            ranges(Key::Signed(i64::MIN), Key::Signed(i64::MAX), u64::MAX),
            [
                IntegerRange {
                    lower: Key::Signed(i64::MIN),
                    upper: Key::Signed(i64::MAX - 1),
                },
                IntegerRange {
                    lower: Key::Signed(i64::MAX),
                    upper: Key::Signed(i64::MAX),
                },
            ]
        );
    }

    #[test]
    fn unsigned_maximum_is_included_without_successor_arithmetic() {
        assert_eq!(
            ranges(Key::Unsigned(u64::MAX - 2), Key::Unsigned(u64::MAX), 2),
            [
                IntegerRange {
                    lower: Key::Unsigned(u64::MAX - 2),
                    upper: Key::Unsigned(u64::MAX - 1),
                },
                IntegerRange {
                    lower: Key::Unsigned(u64::MAX),
                    upper: Key::Unsigned(u64::MAX),
                },
            ]
        );
        assert_eq!(
            ranges(Key::Unsigned(0), Key::Unsigned(u64::MAX), u64::MAX)
                .last()
                .copied(),
            Some(IntegerRange {
                lower: Key::Unsigned(u64::MAX),
                upper: Key::Unsigned(u64::MAX),
            })
        );
    }

    #[test]
    fn sparse_keys_are_covered_once_by_closed_ranges() {
        let keys = [-100, -1, 0, 1, 8, 50, 51, 100];
        let ranges = ranges(Key::Signed(-100), Key::Signed(100), 16);
        for key in keys {
            assert_eq!(
                ranges
                    .iter()
                    .filter(|range| match (range.lower, range.upper) {
                        (Key::Signed(lower), Key::Signed(upper)) => lower <= key && key <= upper,
                        _ => false,
                    })
                    .count(),
                1,
                "sparse key {key} must belong to exactly one range"
            );
        }
    }

    #[test]
    fn small_signed_intervals_form_a_gapless_non_overlapping_partition() {
        for minimum in -8i64..=8 {
            for maximum in minimum..=8 {
                for span in 1..=12 {
                    let partitions = ranges(Key::Signed(minimum), Key::Signed(maximum), span);
                    assert_eq!(partitions.first().unwrap().lower, Key::Signed(minimum));
                    assert_eq!(partitions.last().unwrap().upper, Key::Signed(maximum));
                    for pair in partitions.windows(2) {
                        let (Key::Signed(left), Key::Signed(right)) =
                            (pair[0].upper, pair[1].lower)
                        else {
                            unreachable!();
                        };
                        assert_eq!(left.checked_add(1), Some(right));
                    }
                    for range in &partitions {
                        let (Key::Signed(lower), Key::Signed(upper)) = (range.lower, range.upper)
                        else {
                            unreachable!();
                        };
                        assert!(upper >= lower);
                        assert!((upper as i128 - lower as i128 + 1) <= i128::from(span));
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_or_ambiguous_bounds_are_rejected() {
        assert_eq!(
            IntegerRanges::new(None, None, 0).unwrap_err(),
            RangeError::ZeroSpan
        );
        assert_eq!(
            IntegerRanges::new(Some(Key::Signed(0)), Some(Key::Signed(1)), 0).unwrap_err(),
            RangeError::ZeroSpan
        );
        assert_eq!(
            IntegerRanges::new(Some(Key::Signed(0)), None, 1).unwrap_err(),
            RangeError::IncompleteBounds
        );
        assert_eq!(
            IntegerRanges::new(Some(Key::Signed(0)), Some(Key::Unsigned(1)), 1).unwrap_err(),
            RangeError::MixedSignedness
        );
        assert_eq!(
            IntegerRanges::new(Some(Key::Unsigned(2)), Some(Key::Unsigned(1)), 1).unwrap_err(),
            RangeError::ReversedBounds
        );
    }
}
