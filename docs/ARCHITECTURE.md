# Xaero-rs architecture

Xaero-rs is a single Horizon **native mod** (mixin bundle) that re-implements
Xaero's Minimap + Xaero's World Map as one mod. Horizon native mods are Rust
helpers + modules injected into the game's own source before it compiles
(see Horizon `docs/native-mods.md`).

## Layout

```
mod/
  horizon-mixin.toml      bundle manifest: modules, injections, mod crate
  runtime.rs              module "umbx_xaero" added to crates/horizon-client/src/game.rs
                          all game-touching logic (state, hooks, drawing)
  hooks/*.rs              tiny shims pasted into hooked functions; they call runtime
  engine/                 crate "xaero-rs-engine": game-INDEPENDENT code
                          (Xaero file formats, waypoint model, map tile store, colors)
```

Rule of thumb: anything that needs game types lives in `runtime.rs`; anything
pure (parsing Xaero waypoint files, region unpacking, color math) lives in
`engine/` so it can be built and unit-tested standalone (the mod crate cannot
depend on `horizon-client`, and its dependencies must already be in Horizon's
`Cargo.lock`).

## How the pieces map to Xaero's features

| Xaero feature | Xaero-rs implementation |
|---|---|
| Minimap HUD (terrain) | `native_minimap` replace hook → game renders tiles; our settings choose corner/size/zoom/rotate/round |
| Cave mode | Game picks `Layer::Below` when no sky light (nether/caves). Manual toggle: later mixin into the minimap layer choice |
| Entity radar | `client_entities()` each frame; dots drawn over the minimap in the `hud` hook |
| World Map screen | `native_mod_screen_open("xaero_rs")`; draw in `screen_render_post` (`screen.id == "mod"`, `game.native_mod_screen() == Some(ID)`) |
| Map tiles | Own tile store keyed by chunk, scanned from `game.world` at the `frame` hook (budgeted, nearest-first — same approach as the game's `minimap.rs` `Tile` scan) |
| Map rendering (GPU) | Compose the visible window into the 1024x1024 `mod_image` atlas slot, upload only on change, `sprite` it scaled; overlays (waypoints, grid, entities) as fills/text |
| Pan/zoom | `screen_mouse_*_pre` hooks (cancel = we own the input); drag pans, wheel zooms; `[` `]` zoom the minimap |
| Waypoints in world | `native_world_primitives` → `beam` + camera-facing `text` per enabled waypoint |
| Waypoint management | Drawn inside the map screen; text fields via `screen_char_typed_pre`/`screen_key_pressed_pre` |
| Deathpoints | `native_player_died` hook |
| Keybinds | `NativeMod.keys` (Options > Mods, rebindable) + `native_key` hook + `native_mod_key(ID, name, default)` |
| Settings | Options > Mods page: `native_settings`/`native_settings_click`; persisted with `native_set_setting` |
| Xaero data compatibility | `engine::xaero` reads/writes `XaeroWaypoints/` and `XaeroWorldMap/` folders directly, in the game dir |

## Keybind defaults (Xaero parity)

| Action | Default |
|---|---|
| Open world map | `M` |
| Waypoint menu (create at player) | `Y` |
| Waypoint list | `U` |
| Quick-toggle waypoints | `B` |
| Minimap zoom in/out | `[` / `]` |
| World map zoom | mouse wheel (plus `+`/`-`) |

## Persistence

Settings: `native_set_setting` (instance-scoped, under the mod's data dir).
Map + waypoints: Xaero's own folder layout in the game dir, read on world load
and written on change, so data round-trips with the Java mods untouched.

## Dev workflow

- Code lives in this repo. The VM (`instabox` box `xaero-rs`) holds a clone +
  the Horizon source at `/data/horizon-src` for reference; CI-style engine
  tests run there.
- Building/running happens on the player's machine via the Horizon launcher:
  `horizon-launcher native-build mod/ --instance horizon` — native builds
  require the player's own `--trust-native-code`; an agent must never pass it.
- Push frequently; the build key hashes every bundle file, so an incremental
  rebuild only recompiles `horizon-client` (~1 min with `native-profile dev`).
