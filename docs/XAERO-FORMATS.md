# Xaero's on-disk formats — spec for Xaero-rs compatibility

Distilled from open-source implementations cross-checked against real archives
(xaerotools, conflux-map reference specs, LumilioCL fixture, XaeroPlus). See
the sources: github.com/dekrom/xaerotools, github.com/Conflux-Union/conflux-map
(docs/reference-specs/), github.com/EdwinZhanCN/LumilioCL.

## Waypoints

Roots: `<gameDir>/xaero/minimap/` (24.3.0+) or `XaeroWaypoints/` (legacy).
Layout: `<root>/<worldId>/<dimFolder>/<file>`.

- worldId: singleplayer = save folder name; multiplayer = `Multiplayer_<addr>`.
  Escapes: `_`->`%us%`, `/`->`%fs%`, `\`->`%bs%`; multiplayer strips `:port`
  then `:`->`§`. Case-insensitive match.
- dimFolder: `dim%0` overworld, `dim%-1` nether, `dim%1` end (legacy names
  `Overworld`/`Nether`/`The End` accepted on read).
- Files: `waypoints.txt` (single world) or `<multiworldId>_<name>.txt`.
  Sets are LINES inside the file, not files.

File: UTF-8, `\n`. Header comment `#\n#waypoint:name:initials:x:y:z:color:disabled:type:set:rotate_on_tp:tp_yaw:visibility_type:destination\n#`; optional first line `sets:<a>:<b>...`. Then `waypoint:` lines.

`waypoint:` fields (colon-separated, order fixed):
`waypoint:name:initials:x:y:z:color:disabled:type:set:rotate_on_tp:tp_yaw:visibility_type:destination`
- name/initials: literal `:` stored as `§§`.
- y: i32 or `~` (no Y).
- color: palette index, `max(0,n) mod 16`; 16-19 occur (extra colors).
- disabled: true = hidden. type: 0 normal, 1 latest death, 2 old death.
- set: `gui.xaero_default` default; `gui.xaero_deathpoint[_old]` death sets.
- fields 10-13 optional (min loadable length: 10 tokens).
- `server_waypoint:<key>:<bool>` lines: preserve. Unknown lines: preserve.

## World Map

Roots: `<gameDir>/XaeroWorldMap/` (or `xaero/world-map/` newer).
Layout: `<root>/<worldId>/<dim>/<mwFolder>/<regionX>_<regionZ>.zip`.

- dim folders: overworld literally `null` (alias `DIM0`), nether `DIM-1`,
  end `DIM1`. Singleplayer: regions directly in the dim folder.
- region = 512x512 blocks: `regionX = floor(blockX / 512)`.
- zip contains exactly one entry: `region.xaero` (deflate). Legacy bare
  `<rx>_<rz>.xaero` = same stream uncompressed.
- Ignore: `cache*/` (xwmc render caches), `*_backup_<n>/`, `*.temp`,
  `*.outdated`, `.lock`.
- Metadata: `<dim>/dimension_config.txt` (`MWName:`, `caveModeType:`,
  `dimensionTypeId:` lines); `<world>/server_config.txt`.

### Region stream (big-endian, Java DataOutputStream)

```
u8 0xFF; i32 (major<<16)|minor; [u8 flag if major==2&&minor>=5]
chunkBlock* until EOF (truncation tolerated):
  u8 marker = (chunkX<<4)|chunkZ   (chunkX/Z 0..8; bits 3,7 always 0)
  4x4 tiles (tileX outer, tileZ inner):
    i32 -1                      absent tile
    | 256 pixels + u8 interpVersion + i32 caveStart + u8 caveDepth
      (trailers gated by minor>=4/6/7; caveStart 0x7FFFFFFF = surface)
pixel: i32 params bitfield + gated payload
```

Params bits (minor-8 framing):
- 0 blockstate follows (only clear for grass surface)
- 1 overlay list follows (u8 count <=10, then i32 params + gated state each;
  bit 0 non-water, bits 11-14 opacity 0-15 at minor>=8)
- 2-3 legacy color type (skip)
- 7 is-cave-block hint
- 8-11 light 0-15
- 12-19 height low 8; 25-28 height high 4 -> signed 12-bit (minor>=4)
- 20 biome follows (palette idx u8)
- 21 blockstate inline (new palette entry)
- 22 biome inline (new palette entry, writeUTF string)
- 24 top-height u8 follows (minor>=4)

Palettes: per stream, first-occurrence order; blockstate inline = Java NBT
compound (major<8 keys `Name`/`Properties`; major 8 keys `id`/`properties`);
biome inline = writeUTF.

Water depth = topHeight - height. Void columns: inline air, height = bottom.

Version dispatch: no 0xFF = major 0 (skip); majors 1-4 legacy (skip);
5 (1.17), 6 (1.18+), 7 (1.21.8-26.2), 8 (26.3, `id`/`properties` NBT keys).
**Write target 7.8** — read by everything modern; xaerotools round-trips it
byte-identically.
