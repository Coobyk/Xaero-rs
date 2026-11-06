// Xaero-rs storage: world identity, Xaero-format waypoint files and map
// regions on disk. Compiled into the game as module `umbx_xaero_storage`.
//
// Data layout (docs/XAERO-FORMATS.md): waypoints live under
// `xaero/minimap/<world>/dim%N/waypoints.txt` (new) and `XaeroWaypoints/`
// (legacy) — both are read, both are written. Map regions live under
// `xaero/world-map/` and `XaeroWorldMap/`, as
// `<world>/<dim>/<regionX>_<regionZ>.zip` (overworld's dim folder is
// literally `null`).

use std::cell::RefCell;
use std::path::PathBuf;

use xaero_rs_engine::xaero::regions::{self, Region};
use xaero_rs_engine::xaero::waypoints::{self, WaypointFile};

use crate::game::Game;

thread_local! {
    /// The singleplayer save folder name (captured at `open_world`).
    pub(super) static SP_FOLDER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The game called "open world": remember the save folder.
pub(super) fn open_world(_game: &mut Game<'_>, folder: &str) {
    SP_FOLDER.with_borrow_mut(|f| *f = Some(folder.to_owned()));
}

/// A world closed: save everything for its last dimension.
pub(super) fn world_close(game: &mut Game<'_>) {
    super::umbx_xaero::save_world_data(game);
}

/// Xaero's multiplayer world id: `Multiplayer_<address>` with the port cut,
/// `:` as `§`, `_` as `%us%`, `/` and `\` escaped, trailing dots trimmed.
fn multiplayer_id(address: &str) -> String {
    let host = match address.rfind("]:") {
        // IPv6 literal [::1]:25565
        Some(i) => &address[1..i],
        None => address.split(':').next().unwrap_or(address),
    };
    let host = host.trim_end_matches('.');
    host.replace(':', "§")
        .replace('_', "%us%")
        .replace('/', "%fs%")
        .replace('\\', "%bs%")
}

/// Xaero's singleplayer world id: the save folder with escapes.
fn singleplayer_id(folder: &str) -> String {
    folder
        .replace('_', "%us%")
        .replace('/', "%fs%")
        .replace('\\', "%bs%")
}

/// The Xaero world id (container folder) of the open world.
pub(super) fn world_id(game: &Game<'_>) -> String {
    let address = game
        .native_client_state()
        .and_then(|s| s.server_address)
        .unwrap_or_default();
    if !address.is_empty() && address != "local" {
        format!("Multiplayer_{}", multiplayer_id(&address))
    } else {
        SP_FOLDER
            .with_borrow(|f| f.clone())
            .map(|f| singleplayer_id(&f))
            .unwrap_or_else(|| "Multiplayer_Unknown".to_owned())
    }
}

/// The dimension's number: 0 overworld, -1 nether, 1 end.
pub(super) fn dimension_number(dim: &str) -> i32 {
    match dim {
        "minecraft:overworld" => 0,
        "minecraft:the_nether" => -1,
        "minecraft:the_end" => 1,
        other => other
            .rsplit(':')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    }
}

/// The map region's dimension folder (overworld is literally `null`).
fn map_dim_folder(n: i32) -> String {
    match n {
        0 => "null".to_owned(),
        other => format!("DIM{}", other),
    }
}

/// Roots to read and write, in order [new, legacy].
fn roots(game: &Game<'_>) -> [PathBuf; 2] {
    let dir = super::umbx_xaero::game_dir(game);
    [dir.join("xaero"), dir.join("")]
}

/// The waypoint file for dimension `n`, new and legacy.
pub(super) fn waypoint_paths(game: &Game<'_>, n: i32) -> Vec<PathBuf> {
    let id = world_id(game);
    let dim = format!("dim%{}", n);
    let [new_root, legacy_root] = roots(game);
    vec![
        new_root.join("minimap").join(&id).join(&dim).join("waypoints.txt"),
        legacy_root.join("XaeroWaypoints").join(&id).join(&dim).join("waypoints.txt"),
    ]
}

/// The map region zip for chunk (cx, cz), new and legacy.
pub(super) fn region_paths(game: &Game<'_>, n: i32, rx: i32, rz: i32) -> Vec<PathBuf> {
    let id = world_id(game);
    let dim = map_dim_folder(n);
    let name = format!("{}_{}.zip", rx, rz);
    let [new_root, legacy_root] = roots(game);
    vec![
        new_root.join("world-map").join(&id).join(&dim).join(&name),
        legacy_root.join("XaeroWorldMap").join(&id).join(&dim).join(&name),
    ]
}

/// Reads the dimension's waypoint file. The first root that has one wins.
pub(super) fn load_waypoints(game: &Game<'_>, n: i32) -> Option<WaypointFile> {
    for path in waypoint_paths(game, n) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Some(waypoints::parse(&text));
        }
    }
    None
}

/// Writes the dimension's waypoint file to every root.
pub(super) fn save_waypoints(game: &Game<'_>, n: i32, file: &WaypointFile) {
    let text = waypoints::write(file);
    for path in waypoint_paths(game, n) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, &text);
    }
}

/// Loads a region zip (first root that has one), or None.
pub(super) fn load_region(game: &Game<'_>, n: i32, rx: i32, rz: i32) -> Option<Region> {
    for path in region_paths(game, n, rx, rz) {
        if let Ok(zip) = std::fs::read(&path) {
            let stream = regions::unzip_region(&zip).ok()?;
            return regions::decode_region(&stream).ok();
        }
    }
    None
}

/// Writes a region zip to every root.
pub(super) fn save_region(game: &Game<'_>, n: i32, rx: i32, rz: i32, region: &Region) {
    let stream = regions::encode_region(region);
    let zip = regions::zip_region(&stream);
    for path in region_paths(game, n, rx, rz) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("zip.temp");
        if std::fs::write(&tmp, &zip).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}
