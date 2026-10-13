//! Bounds for a multi-hop walk over the citation graph.

/// The deepest walk a caller may request (1 = just the starting Work).
pub const WALK_DEPTH_MAX: u8 = 3;
/// The widest fanout a caller may request per parent.
pub const WALK_FANOUT_MAX: u8 = 10;
/// The most Works one walk may expand in total.
pub const WALK_EXPANSIONS_MAX: usize = 40;

const WALK_DEPTH_DEFAULT: u8 = 1;
const WALK_FANOUT_DEFAULT: u8 = 5;

/// Why walk bounds were rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WalkBoundsError {
    #[error("depth must be between 1 and {WALK_DEPTH_MAX}")]
    Depth,
    #[error("fanout must be between 1 and {WALK_FANOUT_MAX}")]
    Fanout,
}

impl WalkBoundsError {
    /// The machine-readable API error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Depth => "invalid_depth",
            Self::Fanout => "invalid_fanout",
        }
    }
}

/// Validated depth and fanout for a walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkBounds {
    pub depth: u8,
    pub fanout: u8,
}

impl Default for WalkBounds {
    fn default() -> Self {
        Self {
            depth: WALK_DEPTH_DEFAULT,
            fanout: WALK_FANOUT_DEFAULT,
        }
    }
}

impl WalkBounds {
    /// Validate optional requested bounds, filling defaults for absent ones.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn parse(depth: Option<i64>, fanout: Option<i64>) -> Result<Self, WalkBoundsError> {
        let depth = match depth {
            None => WALK_DEPTH_DEFAULT,
            Some(value) if (1..=i64::from(WALK_DEPTH_MAX)).contains(&value) => value as u8,
            Some(_) => return Err(WalkBoundsError::Depth),
        };
        let fanout = match fanout {
            None => WALK_FANOUT_DEFAULT,
            Some(value) if (1..=i64::from(WALK_FANOUT_MAX)).contains(&value) => value as u8,
            Some(_) => return Err(WalkBoundsError::Fanout),
        };
        Ok(Self { depth, fanout })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_reject_out_of_range_depth_and_fanout_when_parsing_walk_bounds() {
        for bad in [0, 4, 11, -1, i64::MAX, i64::MIN] {
            assert_eq!(
                WalkBounds::parse(Some(bad), None),
                Err(WalkBoundsError::Depth)
            );
        }
        for bad in [0, 11, -1, 256, i64::MAX, i64::MIN] {
            assert_eq!(
                WalkBounds::parse(None, Some(bad)),
                Err(WalkBoundsError::Fanout)
            );
        }
        assert_eq!(WalkBoundsError::Depth.code(), "invalid_depth");
        assert_eq!(WalkBoundsError::Fanout.code(), "invalid_fanout");
        // Depth is checked before fanout.
        assert_eq!(
            WalkBounds::parse(Some(0), Some(0)),
            Err(WalkBoundsError::Depth)
        );
        assert_eq!(
            WalkBounds::parse(Some(3), Some(10)),
            Ok(WalkBounds {
                depth: 3,
                fanout: 10
            })
        );
    }

    #[test]
    fn should_default_depth_one_fanout_five_when_bounds_absent() {
        let parsed = WalkBounds::parse(None, None).expect("defaults");
        assert_eq!(
            parsed,
            WalkBounds {
                depth: 1,
                fanout: 5
            }
        );
        assert_eq!(parsed, WalkBounds::default());
    }
}
