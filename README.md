# Xaero-rs

Xaero's World Map + Xaero's Minimap, rewritten as **one Horizon native mod in
Rust**, faithful to Xaero's features, look, keybinds and data formats.

- Status: vertical slice in development (see `docs/ARCHITECTURE.md`).
- Target: [Horizon](https://playhorizon.dev) (Minecraft in Rust) native mods.

## Layout

- `mod/` — the mixin bundle (`horizon-mixin.toml`, runtime module, hook
  shims, and the game-independent `engine` crate).
- `docs/` — architecture and Horizon API notes.

## Build & install (the player's machine — requires trusting native code)

```sh
horizon-launcher native-build mod/ --instance horizon --trust-native-code
```

or install from GitHub ("Add from GitHub" / `mods add-github`). Native
builds compile the game with the mod's code injected; the first build takes
a few minutes, later ones are incremental.

**Disable the official `minimap` mod** (Options > Mods) so Xaero-rs owns the
minimap.
