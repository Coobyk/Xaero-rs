//! Game-independent core of Xaero-rs. Anything here must compile without
//! Horizon: the mod's crate cannot depend on `horizon-client`, and its
//! dependencies must already be in Horizon's `Cargo.lock`.

pub mod waypoint;
pub mod xaero;

pub use waypoint::{Waypoint, WaypointColor, WAYPOINT_COLORS};

/// The mod's version, as shown in Options > Mods.
pub const VERSION: &str = "0.1.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waypoint_colors_are_opaque_argb() {
        for (i, c) in WAYPOINT_COLORS.iter().enumerate() {
            assert_eq!(c & 0xFF00_0000, 0xFF00_0000, "color {i} not opaque");
        }
    }

    #[test]
    fn waypoint_roundtrip() {
        let w = Waypoint::new("Home", 12, 64, -34, WaypointColor::Red);
        assert_eq!(w.name, "Home");
        assert_eq!(w.pos, [12, 64, -34]);
    }
}
