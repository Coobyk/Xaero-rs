//! The waypoint model, shared by the world map, the minimap and the in-world
//! beams. Field set mirrors what Xaero's waypoint files carry; the file
//! parsers (engine::xaero) convert to/from this.

/// Xaero's 16 waypoint colours, as opaque ARGB. Index order matches Xaero's
/// colour picker cycle (blue, cyan, green, yellow, orange, red, pink, magenta,
/// purple, gray, white, brown, dark red, dark green, dark blue, black).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaypointColor {
    Blue,
    Cyan,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Magenta,
    Purple,
    Gray,
    White,
    Brown,
    DarkRed,
    DarkGreen,
    DarkBlue,
    Black,
}

pub const WAYPOINT_COLORS: [u32; 16] = [
    0xFF00_38DB, // blue
    0xFF00DBDB,  // cyan
    0xFF00DB00,  // green
    0xFFDBDB00,  // yellow
    0xFFDB7500,  // orange
    0xFFDB0000,  // red
    0xFFFF00FF,  // pink
    0xFFDB00DB,  // magenta
    0xFF9B00DB,  // purple
    0xFF7B7B7B,  // gray
    0xFFFFFFFF,  // white
    0xFF8B5A2B,  // brown
    0xFF8B0000,  // dark red
    0xFF006400,  // dark green
    0xFF00008B,  // dark blue
    0xFF141414,  // black
];

impl WaypointColor {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn argb(self) -> u32 {
        WAYPOINT_COLORS[self.index()]
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    /// All variants, in palette order (Xaero's colour cycle).
    pub const ALL: [WaypointColor; 16] = [
        Self::Blue,
        Self::Cyan,
        Self::Green,
        Self::Yellow,
        Self::Orange,
        Self::Red,
        Self::Pink,
        Self::Magenta,
        Self::Purple,
        Self::Gray,
        Self::White,
        Self::Brown,
        Self::DarkRed,
        Self::DarkGreen,
        Self::DarkBlue,
        Self::Black,
    ];
}

/// One waypoint. Positions are block coordinates; `y` is the stored
/// (teleport) Y, which may be outside the world for "any Y" waypoints.
#[derive(Clone, Debug, PartialEq)]
pub struct Waypoint {
    pub name: String,
    /// X, Y, Z in block coordinates.
    pub pos: [i64; 3],
    pub color: WaypointColor,
    pub enabled: bool,
    /// Xaero's symbol field: a letter/digit/emoji character, or one of the
    /// named symbols ("death", "biome-white"...). Empty = the default diamond.
    pub symbol: String,
    /// Waypoint sets ("temp", "death", "custom: ..."); empty = the main set.
    pub set: String,
}

impl Waypoint {
    pub fn new(name: &str, x: i64, y: i64, z: i64, color: WaypointColor) -> Self {
        Self {
            name: name.to_owned(),
            pos: [x, y, z],
            color,
            enabled: true,
            symbol: String::new(),
            set: String::new(),
        }
    }

    /// The centre of the waypoint's block, where beams and markers anchor.
    pub fn anchor(&self) -> [f64; 3] {
        [
            self.pos[0] as f64 + 0.5,
            self.pos[1] as f64,
            self.pos[2] as f64 + 0.5,
        ]
    }
}
