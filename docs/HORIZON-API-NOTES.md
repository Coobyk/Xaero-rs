# Horizon native-mod API notes (verified against source 66f3981f522b24c250b21523a70080965b940424)

Distilled from Horizon `docs/native-mods.md` and the source; exact paths below
are what `runtime.rs` and the hook shims rely on.

## Bundle mechanics

- A bundle = folder with `horizon-mixin.toml`. `[[module]]` adds a `.rs` file
  as an inline module (`mod <name> { <source> }`) into a game file.
  `[[injection]]` pastes a helper fn at a hook point (HEAD runs on entry;
  `cancellable = true` helpers return `core::ops::ControlFlow<()>`, `Break`
  skips the rest of the function). Helper files contain one bare fn.
- `[[crate]]` stages an own Cargo package into the game workspace as a
  dependency of `horizon-client`; it must not depend on Horizon crates; its
  registry deps must already be in Horizon's `Cargo.lock` (version-matched).
- `[compatibility] horizon_revision` pins the Horizon commit. Local builds via
  `horizon-launcher native-build <folder> --instance <name>` (needs the
  player's `--trust-native-code`). GitHub install: repo with a folder holding
  `horizon-mixin.toml` works via "tree" links.

## Game state (inside `crates/horizon-client`)

- `crate::game::Game<'_>`: `world: World` (client chunk copy), `player`
  (`PlayerMotion`, `.position: Vec3`), `yaw/pitch: f32`, `dim: Dim`,
  `spawn: Option<Vec3>`, `mobs/dropped/orbs` join lists, `res: Arc<Resources>`.
- `World` (`crate::world` / horizon-client-render `world.rs`): `block(x,y,z)
  -> BlockStateId`, `light(x,y,z) -> u8` (low block, high sky), `loaded(x,z)`,
  `revision: u64` (bumps on block change), `day_time`, `weather`.
- `Chunk` (`crate::chunk`): `block/biome/light`, `world_surface(x,z,air)`,
  `motion_blocking(x,z,pred)`, sections.
- `Biomes` (`crate::biome`): `list: Vec<BiomeInfo>` with
  `grass/foliage/water: u32` tints, `name`. `Tints::biome_id` in
  `crate::mesher` does vanilla-style blended biome selection.
- Block map colors: `horizon_items::blocks::{MAP_COLORS, StateFacts
  { map_color, fluid, sturdy_full, ... }}`, `game.res.facts() -> &BlockFacts`.
  The game's `crate::minimap::{Tile, Minimap, scan_column, packed_rgba,
  brightness}` is the reference column-scan/color pipeline.
- Client chunks arrive without `native_chunk_load` (server-only). Track with
  `ChunkMap::membership_revision()` / `deltas_since()`.
- Entities: `Host::client_entities() -> Vec<NativeClientEntity{id, position,
  player_name, ...}>`; direct: `game.mobs` (`Mob.kind.id`, `.position`),
  `game.dropped` (items), `game.orbs`.
- Dimension: `game.native_dimension()`; shapes `LevelShape::OVERWORLD/…`.
  Time: `game.native_clock()` / `world.day_time`.
- `Screen::Mod` (a mod screen): opened by `game.native_mod_screen_open(id)`,
  read back with `game.native_mod_screen() -> Option<&str>`; ESC closes it
  unless a key hook cancels.

## Hook signatures used (all in `crate::game::native_hooks`)

- `native_frame(&mut self, elapsed: Duration)` — every frame.
- `native_hud(&mut self, batch: &mut HudBatch, corners: &mut Corners)`.
- `native_key(&mut self, key: &str, pressed: bool)` — any screen; gate on
  `native_playing()`. Names: `key.keyboard.m`, `key.mouse.left`,
  `key.keyboard.left.bracket`, `key.keyboard.keypad.add`, ...
- `native_wheel(&mut self, steps: f32, claimed: &mut bool)`.
- `native_minimap(&self, minimap: &mut Option<NativeMinimap>)` — replace.
  `NativeMinimap { corner: Corner, size: f32, blocks_per_pixel: f32, rotate:
  bool, round: bool }` (`crate::mod_api`, `Corner` from `crate::extras`).
- `native_mods(&mut self, mods: &mut Vec<NativeMod>)` — push
  `NativeMod { id, title, description, keys: Vec<[String;3]> }`.
- `native_settings(&mut self, mod_id, buttons: &mut Vec<String>)` and
  `native_settings_click(&mut self, mod_id, button: usize)`.
- `native_world_primitives(&self, partial: f32, camera: [f64;3],
  primitives: &mut crate::world_primitives::WorldPrimitives)` — builders
  `.beam(base, height, color)`, `.line/.box_outline/.filled_box/.text(at, s,
  color, scale)`; `.through_walls()`, text `.scale_with_distance()`,
  `.background()`; ARGB colors; `MAX_BEAM_HEIGHT = 2048`.
- `native_screen_render_post(&self, batch: &mut HudBatch, screen:
  &NativeScreenView)` — draw over any open screen; our screen has
  `screen.id == "mod"`, `screen.rect` = whole window (GUI px).
- Input pre-hooks (all `cancellable`): `native_screen_mouse_button_pressed_pre
  (screen, button, modifiers, canceled)`, `_released_pre`, 
  `native_screen_mouse_dragged_pre(screen, button, delta: [f32;2],
  modifiers, canceled)`, `native_screen_mouse_scrolled_pre(screen, scroll:
  [f64;2], modifiers, canceled)`, `native_screen_key_pressed_pre(screen, key,
  repeat, modifiers, canceled)`, `native_screen_char_typed_pre(screen, c,
  modifiers, canceled)`.
- `native_player_died` (deathpoints), `native_command` (commands).

## Drawing helpers (Game methods, GUI-pixel space)

- `native_fill(batch, [x,y,w,h], argb)`; `native_sprite(batch, name, src:
  [f32;4], dst: [f32;4]) -> bool` (atlas sprite, e.g. `"horizon/modimage"`);
  `native_text(batch, &str, [f32;2], rgb, shadow)`; `native_text_width(&str)`;
  `native_mod_image(&mut self, w, h, rgba) -> bool` — uploads an up-to
  1024x1024 RGBA into the reserved `horizon/modimage` sprite (upload only on
  change); `native_item`, `native_gui_paint`, `native_gui_tooltip`.
- The whole GUI draws into one `HudBatch` (vertices + atlas patches) that
  wgpu renders — GPU rendering with no wgpu on the mod side.
- Scissor/clipping: `HudBatch::clip_since(start, x0, y0, x1, y1)`.

## Host/settings/storage

- `game.native_on(id)`, `game.native_playing()`,
  `game.native_mod_key(id, name, default) -> String`,
  `game.native_setting(id, key) -> Option<String>`,
  `game.native_set_setting(id, key, value)`,
  `game.native_data_dir(id) -> PathBuf`, `Game::native_config_dir()`.
- `game.native_player() -> Option<NativePlayer { x, y, z, yaw, pitch,
  dimension }>`; `game.native_camera() -> Option<NativeCamera { ... }>`.

## Gotchas

- The official `minimap` mod also answers `native_minimap` — disable it
  (Options > Mods) so Xaero-rs owns the minimap.
- Screen input hooks fire for *any* open screen; check
  `game.native_mod_screen() == Some(ID)` before consuming.
- `native_chunk_load` never fires on a client of a server (client hook list
  only). `client_entities()` covers mobs and remote players but no kind id.
- Rust 1.99.1, edition 2024 idioms available; `is_none_or` etc. fine.
