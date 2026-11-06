//! Xaero-rs runtime, compiled into the game as module `umbx_xaero`
//! (injected into `crates/horizon-client/src/game.rs`).
//!
//! All game-touching state and logic lives here. Hook shims under `hooks/`
//! call into this module. Game-independent code belongs in
//! `xaero_rs_engine` instead.

use std::cell::RefCell;

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
            blocks_per_pixel: 4.0,
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

pub struct State {
    pub minimap: MinimapSettings,
    pub waypoints: Vec<Waypoint>,
    /// Colour index for the next waypoint created with Y.
    pub next_color: usize,
    pub map: Option<MapView>,
}

impl State {
    fn load(game: &crate::game::Game<'_>) -> Self {
        let mut s = Self {
            minimap: MinimapSettings::default(),
            waypoints: Vec::new(),
            next_color: 0,
            map: None,
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
        // A test waypoint so the beam path is visible on first run.
        if let Some(p) = game.native_player() {
            s.waypoints.push(Waypoint::new(
                "Origin",
                p.x as i64,
                p.y as i64,
                p.z as i64,
                WaypointColor::Red,
            ));
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

// --- hooks (called by the shims under hooks/) ---

pub fn key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    if !pressed || !game.native_on(ID) || !game.native_playing() {
        return;
    }
    let open_map = key == game.native_mod_key(ID, KEYS[0][0], KEYS[0][2]);
    let new_wp = key == game.native_mod_key(ID, KEYS[1][0], KEYS[1][2]);
    let list = key == game.native_mod_key(ID, KEYS[2][0], KEYS[2][2]);
    let toggle = key == game.native_mod_key(ID, KEYS[3][0], KEYS[3][2]);
    let zoom_in = key == game.native_mod_key(ID, KEYS[4][0], KEYS[4][2]);
    let zoom_out = key == game.native_mod_key(ID, KEYS[5][0], KEYS[5][2]);
    if !(open_map || new_wp || list || toggle || zoom_in || zoom_out) {
        return;
    }
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
        let p = game.native_player();
        with_mut(game, |s, _game| {
            if let Some(p) = p {
                s.waypoints.push(Waypoint::new(
                    &format!("Waypoint {}", s.waypoints.len() + 1),
                    p.x as i64,
                    p.y as i64,
                    p.z as i64,
                    WaypointColor::from_index(s.next_color),
                ));
                s.next_color = (s.next_color + 1) % WaypointColor::ALL.len();
            }
        });
    } else if toggle {
        with_mut(game, |s, _game| {
            let any_on = s.waypoints.iter().any(|w| w.enabled);
            for w in &mut s.waypoints {
                w.enabled = !any_on;
            }
        });
    } else if zoom_in || zoom_out {
        with_mut(game, |s, game| {
            s.minimap_zoom_step(zoom_in);
            s.save_minimap(game);
        });
    }
}

pub fn frame(game: &mut crate::game::Game<'_>, _elapsed: std::time::Duration) {
    if game.native_mod_screen() != Some(ID) || !game.native_on(ID) {
        return;
    }
    with_mut(game, |_, _| {});
    // Upload the composed map image when it changed. `native_mod_image`
    // needs `&mut self`, so it happens on the frame hook; drawing only
    // reads it. Placeholder terrain: a chunk-parity checkerboard. The real
    // tile pipeline replaces this.
    let span = rect_span();
    STATE.with_borrow_mut(|slot| {
        let Some(s) = slot.as_mut() else { return };
        let Some(map) = &mut s.map else { return };
        if !map.dirty {
            return;
        }
        let img = compose_test(map, span);
        if game.native_mod_image(1024, 1024, &img) {
            map.dirty = false;
        }
    });
}

/// The visible world span in blocks, from the game's own GUI scale
/// (physical pixels / GUI scale). Only the larger side matters for the
/// placeholder composition.
fn rect_span() -> f64 {
    // A conservative 480x300 GUI-pixel window (auto GUI scale keeps the
    // smaller side near 240-480); refined when the real pipeline lands.
    480.0
}

fn compose_test(map: &MapView, span: f64) -> Vec<u8> {
    let mut img = vec![0u8; 1024 * 1024 * 4];
    let x0 = map.center_x - span / 2.0;
    let z0 = map.center_z - span / 2.0;
    let blocks_per_img_px = span / 1024.0;
    for py in 0..1024usize {
        let wz = z0 + py as f64 * blocks_per_img_px;
        let chunk_z = wz.div_euclid(16.0);
        for px in 0..1024usize {
            let wx = x0 + px as f64 * blocks_per_img_px;
            let chunk_x = wx.div_euclid(16.0);
            let inside = ((chunk_x + chunk_z).rem_euclid(2.0)) == 0.0;
            let i = (py * 1024 + px) * 4;
            let (r, g, b) = if inside { (92, 148, 72) } else { (76, 128, 60) };
            img[i] = r;
            img[i + 1] = g;
            img[i + 2] = b;
            img[i + 3] = 255;
        }
    }
    img
}

pub fn minimap(game: &crate::game::Game<'_>, minimap: &mut Option<NativeMinimap>) {
    with(game, |s| *minimap = s.minimap.native());
}

pub fn world_primitives(
    game: &crate::game::Game<'_>,
    _partial: f32,
    _camera: [f64; 3],
    primitives: &mut WorldPrimitives,
) {
    if !game.native_on(ID) {
        return;
    }
    with(game, |s| {
        for w in s.waypoints.iter().filter(|w| w.enabled) {
            let [x, y, z] = w.anchor();
            primitives.beam([x, y, z], 256, w.color.argb());
            primitives
                .text([x, y + 260.0, z], w.name.clone(), w.color.argb(), 1.0)
                .through_walls()
                .scale_with_distance()
                .background();
        }
    });
}

pub fn screen_render_post(
    game: &crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    screen: &NativeScreenView<'_>,
) {
    if !our_screen(game, screen) {
        return;
    }
    let [rx, ry, rw, rh] = screen.rect;
    game.native_fill(batch, screen.rect, 0xC810_1010);

    with(game, |s| {
        let Some(map) = &s.map else { return };
        // Cover the window with the composed image (placeholder pipeline:
        // stretch; the real one uploads window-shaped patches).
        let _ = game.native_sprite(
            batch,
            "horizon/modimage",
            [0.0, 0.0, 1024.0, 1024.0],
            [rx, ry, rw, rh],
        );

        // Waypoint markers.
        let span = rect_span();
        let wx0 = map.center_x - span / 2.0;
        let wz0 = map.center_z - span / 2.0;
        for w in s.waypoints.iter().filter(|w| w.enabled) {
            let px = rx + ((w.pos[0] as f64 - wx0) / span * rw as f64) as f32;
            let py = ry + ((w.pos[2] as f64 - wz0) / span * rh as f64) as f32;
            if px < rx - 8.0 || py < ry - 8.0 || px > rx + rw + 8.0 || py > ry + rh + 8.0 {
                continue;
            }
            game.native_fill(batch, [px - 3.0, py - 3.0, 6.0, 6.0], w.color.argb());
            game.native_text(batch, &w.name, [px + 5.0, py - 4.0], 0xFF_FF_FF, true);
        }

        // Status line: coordinates and scale.
        let status = format!(
            "{:.0}, {:.0}   {:.2} blocks/px   wheel zoom, drag pan, M to close",
            map.center_x, map.center_z, map.blocks_per_pixel
        );
        game.native_text(
            batch,
            &status,
            [rx + 4.0, ry + rh - 14.0],
            0xE0_E0_E0,
            true,
        );
    });
}

pub fn screen_pressed(
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

pub fn screen_released(
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

pub fn screen_dragged(
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

pub fn screen_scrolled(
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
            let factor = (-scroll[0] * 0.15).exp();
            map.blocks_per_pixel = (map.blocks_per_pixel * factor).clamp(0.05, 64.0);
            map.dirty = true;
            *canceled = true;
        }
    });
}

pub fn screen_key(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    key: &str,
    repeat: bool,
    canceled: &mut bool,
) {
    if !our_screen(game, screen) {
        return;
    }
    // ESC stays uncancelled: the game closes the screen.
    if key == "key.keyboard.escape" {
        return;
    }
    with(game, |s| {
        if let Some(map) = &mut s.map {
            let step = |inward: bool| {
                let factor = if inward { 0.8 } else { 1.25 };
                map.blocks_per_pixel = (map.blocks_per_pixel * factor).clamp(0.05, 64.0);
                map.dirty = true;
            };
            match key {
                "key.keyboard.equal" | "key.keyboard.keypad.add" => step(true),
                "key.keyboard.minus" | "key.keyboard.keypad.subtract" => step(false),
                _ => {}
            }
        }
    });
    let _ = repeat;
    *canceled = true;
}

pub fn screen_char(
    game: &crate::game::Game<'_>,
    screen: &NativeScreenView<'_>,
    c: char,
    canceled: &mut bool,
) {
    if !our_screen(game, screen) {
        return;
    }
    // No text fields yet; swallow characters so nothing else takes them.
    let _ = c;
    *canceled = true;
}

pub fn settings(game: &mut crate::game::Game<'_>, mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id != ID {
        return;
    }
    with_mut(game, |s, _game| buttons.extend(s.minimap_labels()));
}

pub fn settings_click(game: &mut crate::game::Game<'_>, mod_id: &str, button: usize) {
    if mod_id != ID {
        return;
    }
    with_mut(game, |s, game| {
        if s.minimap_click(button) {
            s.save_minimap(game);
        }
    });
}

pub fn player_died(game: &mut crate::game::Game<'_>, _player: u128, _cause: Option<&str>) {
    // Deathpoint waypoints land here (compare `player` with the local
    // player's UUID first once we track it).
    let _ = game;
}

pub fn list(mods: &mut Vec<NativeMod>) {
    mods.push(NativeMod {
        id: ID.to_owned(),
        title: format!("{TITLE} {}", xaero_rs_engine::VERSION),
        description: DESCRIPTION.to_owned(),
        keys: KEYS.iter().map(|k| k.map(str::to_owned)).collect(),
    });
}
