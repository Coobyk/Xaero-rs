// Xaero-rs runtime, compiled into the game as module `umbx_xaero`
// (injected into `crates/horizon-client/src/game.rs`).
//
// All game-touching state and logic lives here. Hook shims under `hooks/`
// call into this module. Game-independent code belongs in
// `xaero_rs_engine` instead.

use std::cell::RefCell;

use xaero_rs_engine::xaero::regions::{Pixel as RegionPixel, Region, Tile as RegionTile};
use xaero_rs_engine::xaero::waypoints::{StoredWaypoint, WaypointFile};
use xaero_rs_engine::{Waypoint, WaypointColor};

use horizon_client::mod_api::{Corner, NativeMinimap, NativeMod, NativeScreenView};
use horizon_client::world_primitives::WorldPrimitives;

pub const ID: &str = "xaero_rs";
pub const TITLE: &str = "Xaero-rs";
pub const DESCRIPTION: &str = "Xaero's World Map + Minimap as one mod.";

/// Rebindable keys, in Xaero's defaults (name, label, default).
pub const KEYS: [[&str; 3]; 6] = [
    ["open_map", "Open world map", "key.keyboard.m"],
    ["waypoint_menu", "New waypoint", "key.keyboard.y"],
    ["waypoint_list", "Waypoint list", "key.keyboard.u"],
    ["waypoint_toggle", "Quick-toggle waypoints", "key.keyboard.b"],
    ["zoom_in", "Minimap zoom in", "key.keyboard.left.bracket"],
    ["zoom_out", "Minimap zoom out", "key.keyboard.right.bracket"],
];

/// Minimap corners, in settings cycle order.
const CORNERS: [Corner; 4] = [
    Corner::TopLeft,
    Corner::TopRight,
    Corner::BottomLeft,
    Corner::BottomRight,
];
const CORNER_NAMES: [&str; 4] = ["Top left", "Top right", "Bottom left", "Bottom right"];

/// Minimap side in GUI pixels.
const MINIMAP_SIZES: [f32; 3] = [64.0, 96.0, 128.0];
/// Blocks per minimap pixel.
const MINIMAP_ZOOMS: [f32; 4] = [0.5, 1.0, 2.0, 4.0];

/// The world-map view: where the map looks at and how many blocks one GUI
/// pixel covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapView {
    pub center_x: f64,
    pub center_z: f64,
    /// Blocks per GUI pixel.
    pub blocks_per_pixel: f64,
    /// While a left-drag is panning: the pointer, GUI px.
    pub dragging: Option<[f32; 2]>,
    /// Set when the composed image changed and needs an upload.
    pub dirty: bool,
}

impl MapView {
    fn new(center_x: f64, center_z: f64) -> Self {
        Self {
            center_x,
            center_z,
            blocks_per_pixel: 1.0,
            dragging: None,
            dirty: true,
        }
    }
}

/// Minimap settings, kept as Options > Mods page buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapSettings {
    pub enabled: bool,
    pub corner: usize,
    pub size: usize,
    pub zoom: usize,
    pub rotate: bool,
    pub round: bool,
    /// Show the radar (entity dots).
    pub radar: bool,
}

impl Default for MinimapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            corner: 1, // top right, Xaero's default
            size: 1,   // 96 px
            zoom: 1,   // 1 block per pixel
            rotate: false,
            round: false,
            radar: true,
        }
    }
}

impl MinimapSettings {
    fn native(&self) -> Option<NativeMinimap> {
        if !self.enabled {
            return None;
        }
        Some(NativeMinimap {
            corner: CORNERS[self.corner],
            size: MINIMAP_SIZES[self.size],
            blocks_per_pixel: MINIMAP_ZOOMS[self.zoom],
            rotate: self.rotate,
            round: self.round,
        })
    }
}

// ---------------- map tile store ----------------

/// One 16x16-chunk column scan: final (biome-tinted) map RGB per column,
/// surface height and water depth. Shading is applied at compose time.
pub(super) struct MapTile {
    rgb: [[u8; 3]; 256],
    height: [i16; 256],
    depth: [u8; 256],
    /// The column's surface is water (map colour WATER): shade by depth.
    water: [bool; 256],
    /// The surface block state and biome, for Xaero region export.
    state: [u16; 256],
    biome: [u16; 256],
    scanned: std::time::Instant,
    revision: u64,
}

impl MapTile {
    fn empty() -> Self {
        Self {
            rgb: [[34, 34, 40]; 256],
            height: [0; 256],
            depth: [0; 256],
            water: [false; 256],
            state: [0; 256],
            biome: [0; 256],
            scanned: std::time::Instant::now(),
            revision: 0,
        }
    }
}

/// `MapColor` ids the scanner substitutes for fluids (the game's own).
const WATER: u8 = 12;
const LAVA: u8 = 4;
const RESCAN: std::time::Duration = std::time::Duration::from_secs(1);

/// Xaero-style map colour brightness modifiers (vanilla map shading).
const BRIGHTNESS: [u32; 4] = [180, 220, 255, 135];

pub(super) struct TileStore {
    tiles: std::collections::HashMap<(i32, i32), MapTile>,
    /// Per block state: map colour id and the palette flags (0/1 water, 2
    /// lava, |4 air, |8 sturdy top) - the game minimap's Palette.
    color: Vec<u8>,
    flags: Vec<u8>,
    /// Xaero regions already imported (or found absent) this session.
    imported: std::collections::HashSet<(i32, i32)>,
    /// Regions whose tiles changed since the last save.
    dirty: std::collections::HashSet<(i32, i32)>,
    /// Block name -> state id, built once with the palette: a 35k-entry
    /// linear scan per imported pixel was a second-long hitch per region.
    by_name: std::collections::HashMap<String, u32>,
    /// A region mid-import: (rx, rz, region, next chunk index).
    pending: Option<(i32, i32, Region, usize)>,
}

impl TileStore {
    fn new() -> Self {
        Self {
            tiles: Default::default(),
            color: Vec::new(),
            flags: Vec::new(),
            imported: Default::default(),
            dirty: Default::default(),
            by_name: Default::default(),
            pending: None,
        }
    }

    fn ensure_palette(&mut self, facts: &horizon_client::blocks::BlockFacts) {
        if !self.color.is_empty() {
            return;
        }
        self.color.reserve(facts.states.len());
        self.flags.reserve(facts.states.len());
        for (i, f) in facts.states.iter().enumerate() {
            self.color.push(f.map_color);
            let mut b = match f.fluid {
                horizon_client::blocks::Fluid::None => 0,
                horizon_client::blocks::Fluid::Water => 1,
                horizon_client::blocks::Fluid::Lava => 2,
            };
            if facts.is_air(horizon_world::registry::BlockStateId(i as u32)) {
                b |= 4;
            }
            if f.sturdy_full >> 1 & 1 != 0 {
                b |= 8;
            }
            self.flags.push(b);
        }
        for (i, f) in facts.states.iter().enumerate() {
            self.by_name.insert(f.name.to_string(), i as u32);
        }
    }

    /// Brings the store up to date with the view: live-scans stale loaded
    /// chunks inside the view rect (any zoom), and imports every intersecting
    /// Xaero region once per session. Tiles are never evicted, so zooming
    /// never re-renders. True when any tile's pixels changed.
    fn update(
        &mut self,
        game: &crate::game::Game<'_>,
        vcx: i32,
        vcz: i32,
        half_cx: i32,
        half_cz: i32,
        budget: std::time::Duration,
    ) -> bool {
        if self.color.is_empty() {
            return false;
        }
        let started = std::time::Instant::now();
        let world = &game.world;
        let air = world.air.0;
        let mut changed = false;
        // 1. Xaero regions intersecting the view, imported whole, once —
        //    first, so saved terrain appears without waiting for live scans.
        //    A region import continues over frames (16 chunks a frame): the
        //    per-pixel conversion is real work and must not hitch.
        const IMPORT_CHUNKS_PER_FRAME: usize = 16;
        if self.pending.is_some() {
            let Some((rx, rz, region, next)) = self.pending.take() else {
                return changed;
            };
            let end = (next + IMPORT_CHUNKS_PER_FRAME).min(region.chunks.len());
            for chunk in &region.chunks[next..end] {
                changed |= self.import_region_chunk(game, rx, rz, chunk);
            }
            if end < region.chunks.len() {
                self.pending = Some((rx, rz, region, end));
            }
            return changed;
        }
        let dim = super::umbx_xaero_storage::dimension_number(game.native_dimension());
        let rx0 = (vcx - half_cx).div_euclid(32);
        let rx1 = (vcx + half_cx).div_euclid(32);
        let rz0 = (vcz - half_cz).div_euclid(32);
        let rz1 = (vcz + half_cz).div_euclid(32);
        'regions: for rx in rx0..=rx1 {
            for rz in rz0..=rz1 {
                if !self.imported.insert((rx, rz)) {
                    continue;
                }
                if let Some(region) =
                    super::umbx_xaero_storage::load_region(game, dim, rx, rz)
                {
                    self.pending = Some((rx, rz, region, 0));
                    break 'regions; // resumed next frame
                }
            }
        }
        // 2. Loaded chunks in the view: rescan when the world changed under
        //    them (or scan fresh ones).
        for (key, chunk) in world.chunks.iter() {
            if (key.0 - vcx).abs() > half_cx + 1 || (key.1 - vcz).abs() > half_cz + 1 {
                continue;
            }
            let stale = match self.tiles.get(key) {
                None => true,
                Some(t) => t.revision != world.revision && t.scanned.elapsed() >= RESCAN,
            };
            if !stale {
                continue;
            }
            let tile = self.scan_tile(game, chunk, air, world.revision);
            self.dirty.insert((key.0.div_euclid(32), key.1.div_euclid(32)));
            changed |= self
                .tiles
                .get(key)
                .is_none_or(|t| t.rgb != tile.rgb);
            self.tiles.insert(*key, tile);
            if started.elapsed() >= budget {
                return changed;
            }
        }
        changed
    }

    /// Imports one region chunk's tiles (missing keys only: live scans are
    /// fresher).
    fn import_region_chunk(
        &mut self,
        game: &crate::game::Game<'_>,
        rx: i32,
        rz: i32,
        chunk: &xaero_rs_engine::xaero::regions::Chunk,
    ) -> bool {
        let mut changed = false;
        let ccx = rx * 32 + i32::from(chunk.at.0) * 4;
        let ccz = rz * 32 + i32::from(chunk.at.1) * 4;
        for (ti, tile) in chunk.tiles.iter().enumerate() {
            let Some(t) = tile else { continue };
            let tx = (ti / 4) as i32;
            let tz = (ti % 4) as i32;
            let key = (ccx + tx, ccz + tz);
            if self.tiles.contains_key(&key) {
                continue;
            }
            self.tiles.insert(key, self.convert_tile(game, t));
            changed = true;
        }
        changed
    }

    /// Converts one region tile (x-major pixels) into a store tile
    /// (z-major slots), resolving colours through the game's facts.
    fn convert_tile(
        &self,
        game: &crate::game::Game<'_>,
        t: &RegionTile,
    ) -> MapTile {
        let biomes = game.res.biomes();
        let mut out = MapTile::empty();
        for slot in 0..256usize {
            // Region pixels are x-major; our slots z-major.
            let px: &RegionPixel = &t.pixels[(slot % 16) * 16 + slot / 16];
            out.height[slot] = px.height;
            out.depth[slot] = px
                .top_height
                .map(|top| (i16::from(top) - px.height).max(0) as u8)
                .unwrap_or(0);
            out.water[slot] = px.state.as_deref() == Some("minecraft:water");
            let name = px.state.as_deref().unwrap_or("minecraft:grass_block");
            out.state[slot] = self.color_by_name(name).unwrap_or(0) as u16;
            out.biome[slot] = px
                .biome
                .as_deref()
                .and_then(|b| biomes.index.get(b).copied())
                .unwrap_or(0);
            out.rgb[slot] = self.import_rgb(game, px);
        }
        out.revision = 1;
        out
    }

    /// One chunk column scan with biome tints (Xaero-style colours).
    fn scan_tile(
        &self,
        game: &crate::game::Game<'_>,
        chunk: &std::sync::Arc<horizon_client::chunk::Chunk>,
        air: u32,
        revision: u64,
    ) -> MapTile {
        let mut tile = MapTile::empty();
        let facts = game.res.facts();
        let biomes = game.res.biomes();
        let wx0 = chunk.x * 16;
        let wz0 = chunk.z * 16;
        for z in 0..16i32 {
            for x in 0..16i32 {
                let slot = (z * 16 + x) as usize;
                let top = chunk.world_surface(x, z, |id| id == air);
                let bottom = chunk.min_y();
                let mut y = top;
                let mut color = 0u8;
                let mut depth = 0u8;
                while y > bottom {
                    let state = chunk.get(x, y, z);
                    let (c, flags) = self.get(state);
                    if c == 0 {
                        y -= 1;
                        continue;
                    }
                    let fluid = flags & 3;
                    if fluid != 0 {
                        let mut below = y - 1;
                        while below > bottom && self.get(chunk.get(x, below, z)).1 & 3 != 0 {
                            depth = depth.saturating_add(1);
                            below -= 1;
                        }
                        depth = depth.saturating_add(1);
                        if flags & 8 == 0 {
                            color = if fluid == 2 { LAVA } else { WATER };
                        } else {
                            color = c;
                        }
                    } else {
                        color = c;
                    }
                    break;
                }
                tile.height[slot] = y as i16;
                tile.depth[slot] = depth;
                tile.water[slot] = color == WATER;
                let state_id = chunk.block(x, y, z).0 as u16;
                tile.state[slot] = state_id;
                tile.biome[slot] = chunk.biome(x, y, z);
                tile.rgb[slot] = if color == 0 {
                    [34, 34, 40]
                } else {
                    self.tinted(facts, biomes, chunk, x, y, z, color, wx0, wz0)
                };
            }
        }
        tile.revision = revision;
        tile
    }

    #[inline]
    fn get(&self, id: u32) -> (u8, u8) {
        let i = id as usize;
        (
            self.color.get(i).copied().unwrap_or(0),
            self.flags.get(i).copied().unwrap_or(4),
        )
    }

    /// The final colour of a column: Xaero's map paints the biome colour
    /// itself for grass, foliage and water (not the muted vanilla map colour
    /// multiplied by a tint), and mildly saturates the rest.
    fn surface_rgb(
        &self,
        biomes: &std::sync::Arc<horizon_client::biome::Biomes>,
        name: &str,
        color: u8,
        biome: u16,
        wx: f64,
        wz: f64,
    ) -> [u8; 3] {
        if GRASS_TINTS.contains(&name) {
            return unpack(biomes.grass(biome, wx, wz));
        }
        if name.ends_with("_leaves") || name == "minecraft:vine" {
            return unpack(biomes.get(biome).foliage);
        }
        if name == "minecraft:water" {
            return unpack(biomes.get(biome).water);
        }
        let id = usize::from(color);
        if id == 0 || id >= horizon_client::blocks::MAP_COLORS.len() {
            return [34, 34, 40];
        }
        // Mild saturation boost (+35% away from grey), Xaero's map colours
        // are punchier than the vanilla map item's.
        let [r, g, b] = unpack(horizon_client::blocks::MAP_COLORS[id]);
        let gray = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
        let sat = |c: u8| {
            (gray + (u32::from(c) - gray) * 135 / 100).min(255) as u8
        };
        [sat(r), sat(g), sat(b)]
    }

    /// The final colour of a scanned column.
    fn tinted(
        &self,
        facts: &horizon_client::blocks::BlockFacts,
        biomes: &std::sync::Arc<horizon_client::biome::Biomes>,
        chunk: &horizon_client::chunk::Chunk,
        x: i32,
        y: i32,
        z: i32,
        color: u8,
        wx0: i32,
        wz0: i32,
    ) -> [u8; 3] {
        let state = chunk.block(x, y, z);
        let (name, biome) = match facts.states.get(state.0 as usize) {
            Some(f) => (&*f.name, chunk.biome(x, y, z)),
            None => ("minecraft:grass_block", 0),
        };
        self.surface_rgb(
            biomes,
            name,
            color,
            biome,
            f64::from(wx0 + x),
            f64::from(wz0 + z),
        )
    }
}

fn unpack(col: u32) -> [u8; 3] {
    [(col >> 16 & 255) as u8, (col >> 8 & 255) as u8, (col & 255) as u8]
}

/// Blocks whose map colour follows the biome's grass colour.
const GRASS_TINTS: [&str; 6] = [
    "minecraft:grass_block",
    "minecraft:short_grass",
    "minecraft:tall_grass",
    "minecraft:fern",
    "minecraft:large_fern",
    "minecraft:bush",
];



impl TileStore {
    /// A block state id whose facts carry `name` (hash lookup).
    fn color_by_name(&self, name: &str) -> Option<u8> {
        self.by_name.get(name).map(|i| self.color[*i as usize])
    }

    /// The map RGB for an imported pixel, through the same colours as a
    /// fresh scan.
    fn import_rgb(
        &self,
        game: &crate::game::Game<'_>,
        px: &RegionPixel,
    ) -> [u8; 3] {
        let biomes = game.res.biomes();
        let name = px.state.as_deref().unwrap_or("minecraft:grass_block");
        let color = self.color_by_name(name).unwrap_or(1);
        let biome = px
            .biome
            .as_deref()
            .and_then(|b| biomes.index.get(b).copied())
            .unwrap_or(0);
        self.surface_rgb(biomes, name, color, biome, 0.0, 0.0)
    }

    /// Exports one tile as a region tile (Xaero 7.8 stream data). The
    /// stream's pixels are x-major (`pixel = x*16 + z`); our tiles are
    /// z-major (`slot = z*16 + x`), like the game minimap's.
    fn export_tile(&self, game: &crate::game::Game<'_>, t: &MapTile) -> RegionTile {
        let facts = game.res.facts();
        let biomes = game.res.biomes();
        let mut pixels = vec![RegionPixel::default(); 256];
        for slot in 0..256usize {
            let x = slot % 16;
            let z = slot / 16;
            pixels[x * 16 + z] = RegionPixel {
                height: t.height[slot],
                top_height: Some((t.height[slot] + i16::from(t.depth[slot])).clamp(0, 255) as u8),
                light: 15,
                cave: false,
                state: Some(
                    facts
                        .states
                        .get(usize::from(t.state[slot]))
                        .map(|f| f.name.to_string())
                        .unwrap_or_else(|| "minecraft:grass_block".to_owned()),
                ),
                biome: Some(
                    biomes
                        .list
                        .get(usize::from(t.biome[slot]))
                        .map(|b| b.name.clone())
                        .unwrap_or_else(|| "minecraft:plains".to_owned()),
                ),
            };
        }
        RegionTile {
            pixels,
            interp_version: 1,
            cave_start: i32::MAX,
            cave_depth: 30,
        }
    }

    /// Writes every dirty region to disk (Xaero's roots), clearing the set.
    pub(super) fn save_dirty(&mut self, game: &crate::game::Game<'_>) {
        let dim = super::umbx_xaero_storage::dimension_number(game.native_dimension());
        let dirty: Vec<(i32, i32)> = self.dirty.drain().collect();
        for (rx, rz) in dirty {
            let Some(chunks) = self.chunks_of_region(game, rx, rz) else {
                continue;
            };
            let region = Region {
                chunks,
                truncated: false,
            };
            super::umbx_xaero_storage::save_region(game, dim, rx, rz, &region);
        }
    }

    /// Builds the region's chunk list from the tile store: every 32x32-chunk
    /// square of stored tiles becomes one chunk with 16 tiles.
    fn chunks_of_region(
        &self,
        game: &crate::game::Game<'_>,
        rx: i32,
        rz: i32,
    ) -> Option<Vec<xaero_rs_engine::xaero::regions::Chunk>> {
        let mut chunks = Vec::new();
        for rcx in 0..8i32 {
            for rcz in 0..8i32 {
                let mut tiles: Vec<Option<RegionTile>> = Vec::with_capacity(16);
                let mut any = false;
                for tx in 0..4i32 {
                    for tz in 0..4i32 {
                        let cx = rx * 32 + rcx * 4 + tx;
                        let cz = rz * 32 + rcz * 4 + tz;
                        match self.tiles.get(&(cx, cz)) {
                            Some(t) => {
                                any = true;
                                tiles.push(Some(self.export_tile(game, t)));
                            }
                            None => tiles.push(None),
                        }
                    }
                }
                chunks.push(xaero_rs_engine::xaero::regions::Chunk {
                    at: (rcx as u8, rcz as u8),
                    tiles,
                });
                let _ = any;
            }
        }
        (!chunks.is_empty()).then_some(chunks)
    }
}

pub struct State {
    pub minimap: MinimapSettings,
    /// This dimension's waypoints (order = file order).
    pub waypoints: Vec<StoredWaypoint>,
    /// The waypoint file's sets and preserved lines for this dimension.
    pub wp_file: WaypointFile,
    /// Waypoints for the current dimension were loaded from disk.
    pub wp_loaded: bool,
    /// Colour index for the next waypoint created with Y.
    pub next_color: usize,
    pub map: Option<MapView>,
    /// Explored terrain, per chunk.
    pub store: TileStore,
    /// The composed 1024x1024 map image and whether it needs an upload.
    pub img: Vec<u8>,
    pub needs_upload: bool,
    /// The last drawn window size in GUI pixels (from screen_render_post).
    pub view_w: f32,
    pub view_h: f32,
    /// The dimension the store was built for.
    pub dimension: &'static str,
    /// Seconds since the last periodic save.
    pub save_timer: f32,
    /// The death screen was up last frame (edge detection for deathpoints).
    pub was_dead: bool,
    /// Frame parity flip for the half-rate compose while panning.
    pub frame_parity: bool,
    /// The last position while alive: where a deathpoint goes.
    pub last_alive: Option<[f64; 3]>,
}

impl State {
    fn load(game: &crate::game::Game<'_>) -> Self {
        let mut s = Self {
            minimap: MinimapSettings::default(),
            waypoints: Vec::new(),
            wp_file: WaypointFile::default(),
            wp_loaded: false,
            next_color: 0,
            map: None,
            store: TileStore::new(),
            img: vec![0; 1024 * 1024 * 4],
            needs_upload: false,
            view_w: 480.0,
            view_h: 300.0,
            dimension: game.native_dimension(),
            save_timer: 0.0,
            was_dead: false,
            last_alive: None,
            frame_parity: false,
        };
        let get = |key: &str| game.native_setting(ID, key);
        if get("minimap_enabled").is_some() {
            s.minimap.enabled = get("minimap_enabled").is_none_or(|v| v.trim() != "false");
            s.minimap.corner = get("minimap_corner")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| *v < CORNERS.len())
                .unwrap_or(1);
            s.minimap.size = get("minimap_size")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| *v < MINIMAP_SIZES.len())
                .unwrap_or(1);
            s.minimap.zoom = get("minimap_zoom")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .filter(|v| *v < MINIMAP_ZOOMS.len())
                .unwrap_or(1);
            s.minimap.rotate = get("minimap_rotate").is_none_or(|v| v.trim() != "false");
            s.minimap.round = get("minimap_round").is_some_and(|v| v.trim() == "true");
            s.minimap.radar = get("minimap_radar").is_none_or(|v| v.trim() != "false");
        }
        s
    }

    fn save_minimap(&self, game: &crate::game::Game<'_>) {
        game.native_set_setting(ID, "minimap_enabled", &self.minimap.enabled.to_string());
        game.native_set_setting(ID, "minimap_corner", &self.minimap.corner.to_string());
        game.native_set_setting(ID, "minimap_size", &self.minimap.size.to_string());
        game.native_set_setting(ID, "minimap_zoom", &self.minimap.zoom.to_string());
        game.native_set_setting(ID, "minimap_rotate", &self.minimap.rotate.to_string());
        game.native_set_setting(ID, "minimap_round", &self.minimap.round.to_string());
        game.native_set_setting(ID, "minimap_radar", &self.minimap.radar.to_string());
    }

    fn minimap_labels(&self) -> Vec<String> {
        let m = &self.minimap;
        vec![
            format!("Minimap: {}", on_off(m.enabled)),
            format!("Corner: {}", CORNER_NAMES[m.corner]),
            format!(
                "Size: {}",
                match m.size {
                    0 => "Small",
                    2 => "Large",
                    _ => "Medium",
                }
            ),
            format!("Zoom: {} block(s) per pixel", MINIMAP_ZOOMS[m.zoom]),
            format!("Turn with the player: {}", on_off(m.rotate)),
            format!("Shape: {}", if m.round { "Round" } else { "Square" }),
            format!("Entity radar: {}", on_off(m.radar)),
        ]
    }

    fn minimap_click(&mut self, button: usize) -> bool {
        let m = &mut self.minimap;
        match button {
            0 => m.enabled = !m.enabled,
            1 => m.corner = (m.corner + 1) % CORNERS.len(),
            2 => m.size = (m.size + 1) % MINIMAP_SIZES.len(),
            3 => m.zoom = (m.zoom + 1) % MINIMAP_ZOOMS.len(),
            4 => m.rotate = !m.rotate,
            5 => m.round = !m.round,
            6 => m.radar = !m.radar,
            _ => return false,
        }
        true
    }

    /// Xaero's minimap zoom: `[` zooms in (fewer blocks per pixel).
    fn minimap_zoom_step(&mut self, inward: bool) {
        let z = &mut self.minimap.zoom;
        if inward && *z > 0 {
            *z -= 1;
        } else if !inward && *z + 1 < MINIMAP_ZOOMS.len() {
            *z += 1;
        }
    }
}

/// The biome's display name for the status line (`minecraft:plains` ->
/// "Plains").
fn game_biome_name(game: &crate::game::Game<'_>, b: u16) -> String {
    let name = game
        .res
        .biomes()
        .list
        .get(usize::from(b))
        .map(|i| i.name.clone())
        .unwrap_or_default();
    let short = name.rsplit(':').next().unwrap_or(&name).replace('_', " ");
    let mut c = short.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => short,
    }
}

fn on_off(v: bool) -> &'static str {
    if v { "ON" } else { "OFF" }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(game: &crate::game::Game<'_>, f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with_borrow_mut(|s| {
        let s = s.get_or_insert_with(|| State::load(game));
        f(s)
    })
}

fn with_mut<R>(game: &mut crate::game::Game<'_>, f: impl FnOnce(&mut State, &crate::game::Game<'_>) -> R) -> R {
    STATE.with_borrow_mut(|s| {
        let s = s.get_or_insert_with(|| State::load(game));
        f(s, game)
    })
}

/// Whether the open screen is ours.
fn our_screen(game: &crate::game::Game<'_>, screen: &NativeScreenView<'_>) -> bool {
    screen.id == "mod" && game.native_mod_screen() == Some(ID)
}

// ---------------- entity radar ----------------

/// Xaero-style radar dot colours: players white, hostiles red, items cyan,
/// other mobs green.
fn dot_color(kind: &str) -> u32 {
    let k = kind.strip_prefix("minecraft:").unwrap_or(kind);
    const HOSTILE: [&str; 24] = [
        "zombie", "husk", "drowned", "zombie_villager", "skeleton", "stray", "bogged",
        "creeper", "spider", "cave_spider", "enderman", "witch", "pillager", "vindicator",
        "evoker", "ravager", "vex", "phantom", "blaze", "ghast", "slime", "magma_cube",
        "hoglin", "warden",
    ];
    if HOSTILE.contains(&k) {
        0xE8_3B_3B
    } else if k == "item" {
        0x53_F2_E8
    } else {
        0x7E_B6_51
    }
}

/// One radar dot: a 1 px black outline around a coloured square, Xaero-style.
fn radar_dot(
    game: &crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    at: [f32; 2],
    argb: u32,
    big: bool,
) {
    let (o, i) = if big { (3.0, 2.0) } else { (2.5, 1.5) };
    game.native_fill(
        batch,
        [at[0] - o, at[1] - o, o * 2.0, o * 2.0],
        0xFF_00_00_00,
    );
    game.native_fill(
        batch,
        [at[0] - i, at[1] - i, i * 2.0, i * 2.0],
        0xFF00_0000 | (argb & 0xFF_FF_FF),
    );
}

/// The minimap's on-screen rect. The game places the minimap before the hud
/// hook, so placing the same size again and stepping back one slot recovers
/// its rect (Corners stacks widgets per corner, 2 px margins).
fn minimap_rect(
    _game: &crate::game::Game<'_>,
    corners: &mut horizon_client::extras_hud::Corners,
    batch: &horizon_client::hud::HudBatch,
    corner: Corner,
    size: f32,
) -> [f32; 2] {
    const MARGIN: f32 = 2.0;
    let _ = _game;
    let (mx, my) = corners.place(corner, size, size);
    let x = if corner.left() { MARGIN } else { mx };
    let y = if corner.top() {
        my - size - MARGIN
    } else {
        my + size + MARGIN
    };
    [x, y]
}

/// The radar over the minimap (hud hook, fires only while playing).
pub(super) fn hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    if !game.native_on(ID) {
        return;
    }
    let (corner, size, bpp, rotate, yaw, radar_on) = with(game, |s| {
        (
            CORNERS[s.minimap.corner],
            MINIMAP_SIZES[s.minimap.size],
            f64::from(MINIMAP_ZOOMS[s.minimap.zoom]),
            s.minimap.rotate,
            game.yaw,
            s.minimap.radar,
        )
    });
    if !radar_on {
        return;
    }
    let Some(p) = game.native_player() else { return };
    let [x, y] = minimap_rect(game, corners, batch, corner, size);
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    let half = (size / 2.0 - 3.0) as f64;
    // When the map turns with the player, dots turn the other way.
    let rot = if rotate { -f64::from(yaw) } else { 0.0 };
    let (s, c) = rot.to_radians().sin_cos();
    let mut clamp = |dx: f64, dz: f64| -> Option<[f32; 2]> {
        let len = (dx * dx + dz * dz).sqrt();
        if len > half * 2.0 {
            // Far beyond the map: hidden, not clamped (Xaero keeps the rim
            // clean; the dots hug the edge only when close to it).
            return None;
        }
        let (dx, dz) = if len > half && len > 0.001 {
            let f = half / len;
            (dx * f, dz * f)
        } else {
            (dx, dz)
        };
        Some([(cx as f64 + dx) as f32, (cy as f64 + dz) as f32])
    };
    for m in game.mobs.iter().filter(|m| !m.removed && !m.invisible) {
        let dx = (m.position.x - p.x) / bpp;
        let dz = (m.position.z - p.z) / bpp;
        // Map-north-up: the map's screen up is -Z. Rotate when turning.
        let (rx, rz) = if rotate {
            (dx * c - dz * s, dx * s + dz * c)
        } else {
            (dx, dz)
        };
        let Some(at) = clamp(rx, rz) else { continue };
        let is_player = game.remote_player_uuid(m.id).is_some();
        let argb = if is_player {
            0xFF_FF_FF
        } else {
            dot_color(&m.kind.id)
        };
        radar_dot(game, batch, at, argb, is_player);
    }
    // Dropped items, smaller and cyan.
    for item in game.dropped.iter() {
        let dx = (item.position.x - p.x) / bpp;
        let dz = (item.position.z - p.z) / bpp;
        let (rx, rz) = if rotate {
            (dx * c - dz * s, dx * s + dz * c)
        } else {
            (dx, dz)
        };
        let Some(at) = clamp(rx, rz) else { continue };
        radar_dot(game, batch, at, 0x53_F2_E8, false);
    }
}

// ---------------- world data persistence ----------------

/// The game folder (the working directory): saves, and Xaero's data roots.
pub(super) fn game_dir(_game: &crate::game::Game<'_>) -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_default()
}

impl State {
    /// Marks the composed map image stale.
    pub(super) fn map_dirty(&mut self) {
        if let Some(map) = &mut self.map {
            map.dirty = true;
        }
    }

    /// Loads this dimension's waypoints once (Xaero's file, both roots).
    pub(super) fn ensure_waypoints(&mut self, game: &crate::game::Game<'_>) {
        if self.wp_loaded {
            return;
        }
        self.wp_loaded = true;
        let n = super::umbx_xaero_storage::dimension_number(self.dimension);
        let Some(file) = super::umbx_xaero_storage::load_waypoints(game, n) else {
            return;
        };
        self.wp_file = WaypointFile {
            sets: file.sets.clone(),
            waypoints: Vec::new(),
            extra_lines: file.extra_lines.clone(),
        };
        self.waypoints = file.waypoints;
        // Colour cycling resumes after the file's last colour.
        if let Some(last) = self.waypoints.last() {
            self.next_color = (last.wp.color.index() + 1) % WaypointColor::ALL.len();
        }
    }

    /// Writes this dimension's waypoint file (both roots).
    pub(super) fn save_waypoints(&mut self, game: &crate::game::Game<'_>) {
        let n = super::umbx_xaero_storage::dimension_number(self.dimension);
        let file = WaypointFile {
            sets: self.wp_file.sets.clone(),
            waypoints: self.waypoints.clone(),
            extra_lines: self.wp_file.extra_lines.clone(),
        };
        super::umbx_xaero_storage::save_waypoints(game, n, &file);
    }

    /// Saves everything for the current dimension: waypoints and dirty map
    /// regions.
    pub(super) fn save_world_data(&mut self, game: &crate::game::Game<'_>) {
        if self.wp_loaded {
            self.save_waypoints(game);
        }
        self.store.save_dirty(game);
    }
}

/// The world closed (or a dimension was left): flush everything to disk.
pub(super) fn save_world_data(game: &mut crate::game::Game<'_>) {
    if !game.native_on(ID) {
        return;
    }
    with_mut(game, |s, game| s.save_world_data(game));
}

/// The game opened a singleplayer world: remember its save folder.
pub(super) fn open_world(_game: &mut crate::game::Game<'_>, folder: &str) {
    super::umbx_xaero_storage::open_world(_game, folder);
}

/// A world lifecycle phase arrived.
pub(super) fn world_lifecycle(
    game: &mut crate::game::Game<'_>,
    _context: &crate::mod_api::NativeLifecycleContext,
    phase: &crate::mod_api::WorldPhase,
    _error_text: &str,
) {
    if matches!(
        phase,
        crate::mod_api::WorldPhase::WorldCloseFinal | crate::mod_api::WorldPhase::WorldCloseFailed
    ) {
        save_world_data(game);
    }
}

// --- hooks (called by the shims under hooks/) ---

pub(super) fn key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !pressed || !game.native_on(ID) {
        return;
    }
    // The map toggle works whenever we are in a world (its own screen
    // included); the gameplay keys need the world unobstructed.
    let in_world = game.native_player().is_some();
    if !in_world {
        return;
    }
    let map_open = game.native_mod_screen() == Some(ID);
    let open_map = key == game.native_mod_key(ID, KEYS[0][0], KEYS[0][2]);
    let new_wp = key == game.native_mod_key(ID, KEYS[1][0], KEYS[1][2]);
    let list = key == game.native_mod_key(ID, KEYS[2][0], KEYS[2][2]);
    let toggle = key == game.native_mod_key(ID, KEYS[3][0], KEYS[3][2]);
    let zoom_in = key == game.native_mod_key(ID, KEYS[4][0], KEYS[4][2]);
    let zoom_out = key == game.native_mod_key(ID, KEYS[5][0], KEYS[5][2]);
    if !(open_map || new_wp || list || toggle || zoom_in || zoom_out) {
        return;
    }
    let playing = game.native_playing() || map_open;
    if open_map || list {
        // M toggles the map; the first open anchors the view on the player.
        let open = game.native_mod_screen() == Some(ID);
        if open {
            game.native_mod_screen_close(ID);
            return;
        }
        let at = game.native_player().map(|p| (p.x, p.z));
        with_mut(game, |s, _game| {
            if s.map.is_none() {
                s.map = Some(MapView::new(
                    at.map(|a| a.0).unwrap_or(0.0),
                    at.map(|a| a.1).unwrap_or(0.0),
                ));
            }
            s.map.as_mut().unwrap().dirty = true;
        });
        game.native_mod_screen_open(ID);
    } else if new_wp {
        if !playing {
            return;
        }
        let p = game.native_player();
        with_mut(game, |s, game| {
            if let Some(p) = p {
                s.ensure_waypoints(game);
                let mut wp = Waypoint::new(
                    &format!("Waypoint {}", s.waypoints.len() + 1),
                    p.x as i64,
                    p.y as i64,
                    p.z as i64,
                    WaypointColor::from_index(s.next_color),
                );
                wp.symbol =
                    xaero_rs_engine::xaero::waypoints::default_initials(&wp.name);
                s.waypoints.push(StoredWaypoint::simple(wp));
                s.next_color = (s.next_color + 1) % WaypointColor::ALL.len();
                s.save_waypoints(game);
            }
        });
    } else if toggle {
        if !playing {
            return;
        }
        with_mut(game, |s, game| {
            s.ensure_waypoints(game);
            let any_on = s.waypoints.iter().any(|w| w.wp.enabled);
            for w in &mut s.waypoints {
                w.wp.enabled = !any_on;
            }
            s.save_waypoints(game);
        });
    } else if (zoom_in || zoom_out) && game.native_playing() {
        with_mut(game, |s, game| {
            s.minimap_zoom_step(zoom_in);
            s.save_minimap(game);
        });
    }
}

pub(super) fn frame(game: &mut crate::game::Game<'_>, _elapsed: std::time::Duration) {
    if !game.native_on(ID) || game.native_player().is_none() {
        return;
    }
    let mut compose_upload = false;
    with_mut(game, |s, game| {
        s.store.ensure_palette(game.res.facts());
        // Dimension changed: flush the old dimension's data, drop the store.
        let dim = game.native_dimension();
        if dim != s.dimension {
            s.save_world_data(game);
            s.dimension = dim;
            s.store.tiles.clear();
            s.store.imported.clear();
            s.store.dirty.clear();
            s.store.pending = None;
            s.wp_loaded = false;
            s.waypoints.clear();
            s.wp_file = WaypointFile::default();
            s.needs_upload = true;
            if let Some(map) = &mut s.map {
                map.dirty = true;
            }
        }
        s.ensure_waypoints(game);
        // Deathpoints: remember where we were alive; the death screen coming
        // up records one there. Covers singleplayer, LAN and servers alike.
        match game.native_player() {
            Some(p) if game.screen != crate::game::Screen::Dead => {
                s.last_alive = Some([p.x, p.y, p.z]);
            }
            _ => {}
        }
        let dead = game.screen == crate::game::Screen::Dead;
        if dead && !s.was_dead
            && let Some(at) = s.last_alive
        {
            let mut wp = Waypoint::new(
                "Death",
                at[0] as i64,
                at[1] as i64,
                at[2] as i64,
                WaypointColor::from_index(0),
            );
            wp.symbol = "D".to_owned();
            wp.set = "gui.xaero_deathpoint".to_owned();
            let mut sw = StoredWaypoint::simple(wp);
            sw.kind = 1;
            sw.visibility = 1;
            sw.destination = true;
            // The previous latest death becomes an old one.
            for old in &mut s.waypoints {
                if old.kind == 1 {
                    old.kind = 2;
                    old.wp.set = "gui.xaero_deathpoint_old".to_owned();
                    old.wp.name = "Death (old)".to_owned();
                }
            }
            s.waypoints.insert(0, sw);
            s.save_waypoints(game);
        }
        s.was_dead = dead;
        // Periodic save, so a crash loses at most half a minute of exploring.
        s.save_timer += 0.5;
        if s.save_timer >= 30.0 {
            s.save_timer = 0.0;
            s.save_world_data(game);
        }
        // The view rect in chunks; the whole saved map renders at any zoom.
        let (vcx, vcz, half_cx, half_cz) = match &s.map {
            Some(map) => {
                let vcx = map.center_x.div_euclid(16.0) as i32;
                let vcz = map.center_z.div_euclid(16.0) as i32;
                let half_cx = ((f64::from(s.view_w) * map.blocks_per_pixel / 2.0 / 16.0).ceil() as i32 + 1).clamp(1, 512);
                let half_cz = ((f64::from(s.view_h) * map.blocks_per_pixel / 2.0 / 16.0).ceil() as i32 + 1).clamp(1, 512);
                (vcx, vcz, half_cx, half_cz)
            }
            None => {
                let at = game.native_player().map(|p| (p.x, p.z));
                (
                    at.map(|a| a.0.div_euclid(16.0) as i32).unwrap_or(0),
                    at.map(|a| a.1.div_euclid(16.0) as i32).unwrap_or(0),
                    8,
                    8,
                )
            }
        };
        if s.store.update(game, vcx, vcz, half_cx, half_cz, std::time::Duration::from_micros(2500)) {
            if let Some(map) = &mut s.map {
                map.dirty = true;
            }
        }
        // Compose and upload while our screen is open; `native_mod_image`
        // needs `&mut self`, so this is the one place it can happen. While a
        // drag pans the map, compose at half rate: a 1024x1024 compose plus
        // its 4 MiB upload every frame is the FPS drop the map used to have.
        s.frame_parity = !s.frame_parity;
        let dragging = s.map.as_ref().is_some_and(|m| m.dragging.is_some());
        let dirty = s.map.as_ref().is_some_and(|m| m.dirty);
        let due = !dragging || s.frame_parity;
        if game.native_mod_screen() == Some(ID) && dirty && due {
            compose(s, s.view_w.max(320.0), s.view_h.max(200.0));
            compose_upload = true;
        }
    });
    if compose_upload {
        STATE.with_borrow_mut(|slot| {
            let Some(s) = slot.as_mut() else { return };
            if game.native_mod_image(1024, 1024, &s.img) {
                s.needs_upload = false;
                if let Some(map) = &mut s.map {
                    map.dirty = false;
                }
            }
        });
    }
}

/// Composes the visible window (`w` x `h` GUI pixels, view `s.map`) into the
/// 1024x1024 image: nearest tile sampling with vanilla-map height and
/// water-depth shading.
fn compose(s: &mut State, w: f32, h: f32) {
    let Some(map) = s.map else { return };
    let State { img, store, .. } = s;
    let img = &mut img[..];
    let span_x = f64::from(w) * map.blocks_per_pixel;
    let span_z = f64::from(h) * map.blocks_per_pixel;
    let x0 = map.center_x - span_x / 2.0;
    let z0 = map.center_z - span_z / 2.0;
    let per_px_x = span_x / 1024.0;
    let per_px_z = span_z / 1024.0;
    let mut last_key: (i32, i32) = (i32::MAX, i32::MAX);
    let mut tile: Option<&MapTile> = None;
    for py in 0..1024usize {
        let wz = z0 + (py as f64 + 0.5) * per_px_z;
        let cz = wz.div_euclid(16.0) as i32;
        let lz = wz.rem_euclid(16.0) as usize;
        for px in 0..1024usize {
            let wx = x0 + (px as f64 + 0.5) * per_px_x;
            let cx = wx.div_euclid(16.0) as i32;
            let key = (cx, cz);
            if key != last_key {
                last_key = key;
                tile = store.tiles.get(&key);
            }
            let i = (py * 1024 + px) * 4;
            let Some(t) = tile else {
                img[i] = 34;
                img[i + 1] = 34;
                img[i + 2] = 40;
                img[i + 3] = 255;
                continue;
            };
            let lx = wx.rem_euclid(16.0) as usize;
            let slot = lz * 16 + lx;
            let [r, g, b] = t.rgb[slot];
            let height = t.height[slot];
            // North neighbour for the slope shading; the tile above when the
            // column is the chunk's north edge.
            let north = if lz > 0 {
                t.height[(lz - 1) * 16 + lx]
            } else {
                store
                    .tiles
                    .get(&(cx, cz - 1))
                    .map(|n| n.height[15 * 16 + lx])
                    .unwrap_or(height)
            };
            let parity = (px ^ py) as i32;
            let bright = brightness_idx(t.water[slot], t.depth[slot], height, north, parity);
            let m = u32::from(BRIGHTNESS[bright]);
            img[i] = ((u32::from(r) * m) / 255) as u8;
            img[i + 1] = ((u32::from(g) * m) / 255) as u8;
            img[i + 2] = ((u32::from(b) * m) / 255) as u8;
            img[i + 3] = 255;
        }
    }
}

/// The game minimap's brightness index: water by depth, land by the slope
/// towards the lower north neighbour, dithered.
fn brightness_idx(is_water: bool, depth: u8, height: i16, north: i16, parity: i32) -> usize {
    let parity = f64::from(parity & 1);
    if is_water {
        let diff = f64::from(depth) * 0.1 + parity * 0.2;
        return if diff < 0.5 {
            2
        } else if diff > 0.9 {
            0
        } else {
            1
        };
    }
    let diff = f64::from(height - north) * 4.0 / 5.0 + (parity - 0.5) * 0.4;
    if diff > 0.6 {
        2
    } else if diff < -0.6 {
        0
    } else {
        1
    }
}

pub(super) fn minimap(game: &crate::game::Game<'_>, minimap: &mut Option<NativeMinimap>) {
    with(game, |s| *minimap = s.minimap.native());
}

pub(super) fn world_primitives(
    game: &crate::game::Game<'_>,
    _partial: f32,
    _camera: [f64; 3],
    primitives: &mut WorldPrimitives,
) {
    if !game.native_on(ID) {
        return;
    }
    with(game, |s| {
        for w in s.waypoints.iter().filter(|w| w.wp.enabled) {
            let [x, y, z] = w.wp.anchor();
            primitives.beam([x, y, z], 256, w.wp.color.argb());
            primitives
                .text([x, y + 260.0, z], w.wp.name.clone(), w.wp.color.argb(), 1.0)
                .through_walls()
                .scale_with_distance()
                .background();
        }
    });
}

pub(super) fn screen_render_post(
    game: &crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    screen: &NativeScreenView<'_>,
) {
    if !our_screen(game, screen) {
        return;
    }
    let [rx, ry, rw, rh] = screen.rect;
    game.native_fill(batch, screen.rect, 0xFF10_1014);

    // A changed window size invalidates the composed image; recompose next
    // frame and skip this one's (stale-projection) terrain.
    let resized = {
        let mut r = false;
        with(game, |s| {
            r = (s.view_w - batch.width).abs() > 0.5 || (s.view_h - batch.height).abs() > 0.5;
            if r {
                s.view_w = batch.width;
                s.view_h = batch.height;
                s.map_dirty();
            }
        });
        r
    };
    if resized {
        return;
    }
    with(game, |s| {
        let Some(map) = &s.map else { return };
        // Cover the window with the composed terrain image.
        let _ = game.native_sprite(
            batch,
            "horizon/modimage",
            [0.0, 0.0, 1024.0, 1024.0],
            [rx, ry, rw, rh],
        );

        // Waypoint markers, in the same projection the image used.
        let span_x = f64::from(batch.width) * map.blocks_per_pixel;
        let span_z = f64::from(batch.height) * map.blocks_per_pixel;
        let wx0 = map.center_x - span_x / 2.0;
        let wz0 = map.center_z - span_z / 2.0;
        for w in s.waypoints.iter().filter(|w| w.wp.enabled) {
            let px = rx + ((w.wp.pos[0] as f64 - wx0) / span_x * rw as f64) as f32;
            let py = ry + ((w.wp.pos[2] as f64 - wz0) / span_z * rh as f64) as f32;
            if px < rx - 8.0 || py < ry - 8.0 || px > rx + rw + 8.0 || py > ry + rh + 8.0 {
                continue;
            }
            game.native_fill(batch, [px - 3.0, py - 3.0, 6.0, 6.0], w.wp.color.argb());
            game.native_text(batch, &w.wp.name, [px + 5.0, py - 4.0], 0xFF_FF_FF, true);
        }

        // Radar dots on the map, in the same projection as the markers.
        if s.minimap.radar {
            let dot = |batch: &mut horizon_client::hud::HudBatch, wx: f64, wz: f64, argb: u32, big: bool| {
                let px2 = rx + ((wx - wx0) / span_x * rw as f64) as f32;
                let py2 = ry + ((wz - wz0) / span_z * rh as f64) as f32;
                if px2 < rx - 4.0 || py2 < ry - 4.0 || px2 > rx + rw + 4.0 || py2 > ry + rh + 4.0 {
                    return;
                }
                radar_dot(game, batch, [px2, py2], argb, big);
            };
            for m in game.mobs.iter().filter(|m| !m.removed && !m.invisible) {
                let is_player = game.remote_player_uuid(m.id).is_some();
                let argb = if is_player { 0xFF_FF_FF } else { dot_color(&m.kind.id) };
                dot(batch, m.position.x, m.position.z, argb, is_player);
            }
            for item in game.dropped.iter() {
                dot(batch, item.position.x, item.position.z, 0x53_F2_E8, false);
            }
        }

        // Status line: the pointer's world position and biome when it is on
        // the map, else the view centre; scale on the right.
        let (mut at_x, mut at_z) = (map.center_x, map.center_z);
        let [moux, mouy] = screen.mouse;
        let on_map = moux >= rx && moux < rx + rw && mouy >= ry && mouy < ry + rh;
        if on_map {
            at_x = wx0 + (f64::from(moux - rx) / f64::from(rw)) * span_x;
            at_z = wz0 + (f64::from(mouy - ry) / f64::from(rh)) * span_z;
        }
        let mut status = format!("{:.0}, {:.0}", at_x, at_z);
        if on_map {
            let b = s
                .store
                .tiles
                .get(&(at_x.div_euclid(16.0) as i32, at_z.div_euclid(16.0) as i32))
                .map(|t| t.biome[(at_z.rem_euclid(16.0) as usize) * 16 + (at_x.rem_euclid(16.0) as usize)]);
            if let Some(b) = b {
                let name = game_biome_name(game, b);
                status.push_str(&format!("  {name}"));
            }
        }
        status.push_str(&format!("   {:.2} blocks/px", map.blocks_per_pixel));
        game.native_text(
            batch,
            &status,
            [rx + 4.0, ry + rh - 14.0],
            0xE0_E0_E0,
            true,
        );
    });
}

pub(super) fn screen_pressed(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    if !our_screen(game, screen) || button != 0 {
        return;
    }
    with(game, |s| {
        if let Some(map) = &mut s.map {
            map.dragging = Some(screen.mouse);
            *canceled = true;
        }
    });
}

pub(super) fn screen_released(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    if !our_screen(game, screen) || button != 0 {
        return;
    }
    with(game, |s| {
        if let Some(map) = &mut s.map {
            if map.dragging.take().is_some() {
                *canceled = true;
            }
        }
    });
}

pub(super) fn screen_dragged(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    button: i32,
    delta: [f32; 2],
    canceled: &mut bool,
) {
    if !our_screen(game, screen) || button != 0 {
        return;
    }
    with(game, |s| {
        if let Some(map) = &mut s.map {
            if map.dragging.is_some() {
                // Move the map with the pointer: dragging right moves the
                // view west (smaller x).
                map.center_x -= delta[0] as f64 * map.blocks_per_pixel;
                map.center_z -= delta[1] as f64 * map.blocks_per_pixel;
                map.dirty = true;
                *canceled = true;
            }
        }
    });
}

pub(super) fn screen_scrolled(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    scroll: [f64; 2],
    canceled: &mut bool,
) {
    if !our_screen(game, screen) {
        return;
    }
    with(game, |s| {
        if let Some(map) = &mut s.map {
            // Scroll up zooms in (fewer blocks per pixel), like Xaero.
            // The wheel is the Y axis of the scroll event.
            let factor = (-scroll[1] * 0.15).exp();
            map.blocks_per_pixel = (map.blocks_per_pixel * factor).clamp(0.05, 64.0);
            map.dirty = true;
            *canceled = true;
        }
    });
}

pub(super) fn screen_key(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    key: &str,
    repeat: bool,
    canceled: &mut bool,
) {
    if !our_screen(game, screen) {
        return;
    }
    let _ = repeat;
    // Cancel ONLY what we handle. A cancelled press never reaches the
    // screens or `native_key`, so swallowing everything here would eat our
    // own toggles (M, Y, U, B). ESC passes so the game closes the screen.
    let zoom_in = matches!(key, "key.keyboard.equal" | "key.keyboard.keypad.add");
    let zoom_out = matches!(key, "key.keyboard.minus" | "key.keyboard.keypad.subtract");
    if !(zoom_in || zoom_out) {
        return;
    }
    *canceled = true;
    with(game, |s| {
        if let Some(map) = &mut s.map {
            let factor = if zoom_in { 0.8 } else { 1.25 };
            map.blocks_per_pixel = (map.blocks_per_pixel * factor).clamp(0.05, 64.0);
            map.dirty = true;
        }
    });
}

pub(super) fn screen_char(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    c: char,
    canceled: &mut bool,
) {
    if !our_screen(game, screen) {
        return;
    }
    // No text fields yet; nothing to take the characters, so let them pass.
    let _ = c;
}

pub(super) fn settings(game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != ID {
        return;
    }
    with_mut(game, |s, _game| buttons.extend(s.minimap_labels()));
}

pub(super) fn settings_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    with_mut(game, |s, game| {
        if s.minimap_click(button) {
            s.save_minimap(game);
        }
    });
}

pub(super) fn player_died(_game: &mut crate::game::Game<'_>, _player: u128, _cause: Option<&str>) {
    // Deathpoints are recorded client-side in the frame hook (the death
    // screen's edge), which covers servers too; the server-side hook has no
    // client position to record.
}

pub(super) fn list(mods: &mut Vec<NativeMod>) {
    mods.push(NativeMod {
        id: ID.to_owned(),
        title: format!("{TITLE} {}", xaero_rs_engine::VERSION),
        description: DESCRIPTION.to_owned(),
        keys: KEYS.iter().map(|k| k.map(str::to_owned)).collect(),
    });
}
