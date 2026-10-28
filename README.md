# Darksiders 2 Modkit

Toolkit for modding a local, legally-owned Darksiders II Deathinitive Edition
installation (single-player, offline).

## Rust toolkit

Prereqs: Rust 1.9x (`cargo`). The game's `media/` folder is located
automatically at `Game/Darksiders II Deathinitive Edition/media`, or via the
`DS2_MEDIA` env var.

```sh
cargo build --release
cargo test --release                 # unit tests + real-file round-trip tests
cargo run --release -- list          # list manifest assets
cargo run --release -- extract characters/death
cargo run --release -- repack  characters/death
cargo run --release -- upak-replace characters/death media/_modkit_work/death_repacked.obpk  # any size
cargo run --release -- obsp-list Brute
cargo run --release -- obsp-show Brute       # parsed properties of a class
cargo run --release -- obsp-show Z2_LD_EncounterList
cargo run --release -- obsp-set Brute ExperienceValue 999   # in-place patch (creates .bak)
cargo run --release -- obsp-set Z2_LD_EncounterList "Encounters[0].Waves[0].Spawns[0].SpawnCountMin" 5
cargo run --release -- obsp-set PossessedBehavior "XPNeededByLevel[2]" 15000   # map element (int key)
cargo run --release -- obsp-set MapManager "MapParams[Fullmap].ViewWidth" 40000 # map element (string key)
cargo run --release -- obsp-dump Brute       # payload -> media/_modkit_work/obsp_Brute.bin
cargo run --release -- obsp-import Brute media/_modkit_work/obsp_Brute.bin  # replace payload (any size)
cargo run --release -- apply mods/example-tougher-brutes/mod.toml          # apply a mod
cargo run --release -- hash ExperienceValue        # crc64 of a name (game hash)
cargo run --release -- obsp-add-prop Brute MaxSimultaneousSpawns 6  # add a new property
cargo run --release -- obpk-unchunk media/_modkit_work/armor.obpk media/_modkit_work/armor.dec
cargo run --release -- obpk-repack media/_modkit_work/armor.obpk media/_modkit_work/armor.dec out.obpk --exact
cargo run --release -- obpk-probe out.obpk         # chunk/stream sanity check
cargo run --release -- armor-body CJ.obj media/_modkit_work/armor.dec media/_modkit_work/armor.dec.stock
cargo run --release -- head-remap media/_modkit_work/death.dec media/_modkit_work/death.dec.stock
cargo run --release -- dds-extract media/_modkit_work/armor.dec media/_modkit_work/armor_textures
cargo run --release -- dds-inject  media/_modkit_work/armor.dec media/_modkit_work/armor_textures
cargo run --release -- tex-bake stock.dds out.dds atlas.rgba      # or: --fill 128,128,255
cargo run --release --bin ds2-modkit-gui   # graphical toolkit
cargo run --release -- world-dump            # worlds.bin + worldnodes64 overview
cargo run --release -- world-show z2_world   # world tree (regions/sky)
cargo run --release -- world-objects z2_world spawnpoint 10  # placed objects
cargo run --release -- world-get 0xFF1A8A 11804 "Root.Objects[0].Position[2]"
cargo run --release -- world-move 0xFF1A8A 11804 "Root.Objects[0]" X Y Z [--write]
cargo run --release -- room-materials environments/world_z1_ld_rm00 [out.obj]  # batch -> material colors (+OBJ/MTL)
```

The GUI has a **Map tab** (worlds, placed objects in a 3D view, edit
X/Y/Z/rotation, Save worlds.bin with a `.bak`, room meshes with per-batch
material colors via the **Materials** button) and a TCP control channel on
`127.0.0.1:47777` (`DS2_GUI_PORT` overrides) for automation:
`powershell -File tools/gui-cmd.ps1 -Command "world z2_world"` /
`"shot out.png"`. Commands: `tab world classes class only all list select
move moveby rotate revert revertall save fit camera target selected props
info log shot meshload meshclear meshtoggle meshinfo meshworld meshprops
meshmats meshtex`. `tools/gui-screenshot.ps1` / `tools/gui-input.ps1` capture
and click the window.

Chunked OBPK payloads (e.g. `items/armor`) are `[u32 usize][zlib]` sequences.
`obpk-repack --exact` keeps every chunk's compressed size (padding with empty
deflate blocks) so the rebuilt file has the exact stock size — the engine
rejects re-chunked armor payloads otherwise. `armor-body` / `head-remap`
remap a replacement mesh's vertices into stock vertex slots so the per-index
skin binding stays coherent.

A mod is a folder with a `mod.toml`:

```toml
name = "example: tougher brutes"

[[patch]]
entry = "Brute"
property = "ExperienceValue"
value = "100"

[[import]]
entry = "Brute_NoXP"
file = "payloads/brute_noxp.bin"   # relative to mod.toml

[[asset]]
asset = "characters/death"
file = "payloads/death.obpk"       # replaces the asset inside media.upak
```

Multiple `[[asset]]` entries are applied in a single pak rewrite. `apply` is
all-or-nothing: every patch/import/asset file is resolved first, then
`scripts.obsp` is rebuilt once and assets are injected, each rewritten file
getting a one-time `.bak` backup.

Work products land in `media/_modkit_work/`: carved `.obpk`, decompressed
`.dec`, extracted `_textures/`, and `<name>_repacked.obpk` that can be placed
back into the game (see docs for the upak/manifest repack step).

## Layout

- `src/pack/manifest.rs` — `manifest.bin` v13 parser (paths, packages, file entries)
- `src/pack/upak.rs` — carve assets out of `media.upak` / `maps.upak`, resize-safe asset replace (manifest offsets patched)
- `src/pack/obpk.rs` — OBPK container decompress/compress, chunked payload scan/unchunk/repack (`--exact`)
- `src/pack/dds.rs` — DDS find/size/extract/inject (same-size texture replacement)
- `src/pack/texbake.rs` — BC1/BC3 DDS re-baker (raw RGBA or flat fill, box-filter mips)
- `src/characters/mesh_body.rs` — CJ body writer: stock-slot vertex remap + UV atlas + arm repose
- `src/characters/mesh_head.rs` — CJ head (mask / maskless) stock-slot vertex remap
- `src/obsp.rs` + `src/obsp/container.rs` — scripts.obsp container: name table, entry index, payload access, resize-safe rebuild
- `src/obsp/payload.rs` — payload property parser (v1 + BOD families) + in-place value patcher
- `src/dsave.rs` + `src/dsave/{model,parse,write,items}.rs` — `.dsav` save parse/write + item summaries/edits
- `src/modkit.rs` — mod manifest (`mod.toml`) loader + transactional apply, asset extract helper
- `src/map/worldmap.rs` — `worlds.bin` / `worldnodes64.bin` parsers + placed-object enumeration (world-space Position/Rotation)
- `src/map/roommesh.rs` — room payload (`environments/world_*`) glomm batch decoder (positions/indices/UVs)
- `src/map/propmesh.rs` — media env asset prop meshes + material colors
- `src/map/roommat.rs` — room batch -> material binding by triangle-shape fingerprints against zone media source meshes
- `src/bin/ds2-modkit-gui.rs` — egui desktop GUI (mods / map / assets / status) + control server
- `src/cursor.rs` — shared little-endian byte reader
- `mods/` — example mods (`example-tougher-brutes`, staged `verify-*` mods)
- `tools/verify-in-game.ps1` — apply/restore staged in-game verification mods
- `tools/` — earlier Python research tools (kept for reference)
