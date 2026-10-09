//! Parsers for Just Cause 2's data formats (Avalanche Engine 2.0, PC).
//!
//! | Module      | Format                                             |
//! |-------------|----------------------------------------------------|
//! | `hash`      | lookup3 name hashing used for every file/property  |
//! | `tab`       | `.tab` archive index                               |
//! | `vfs`       | `GameFs`: archives + DLC + dropzone as one tree    |
//! | `sarc`      | SARC small archives (`.ee/.bl/.nl/.fl`)            |
//! | `compress`  | zlib wrappers (`.eez/.blz/.nlz/.flz`)              |
//! | `property`  | binary property containers (`.bin/.asb/.epe/...`)  |
//! | `detect`    | identify a blob by magic                           |
//! | `names`     | hash → name tables, incl. harvesting from the data |
//! | `rbm`       | RBMDL render-block models (vehicles, props, characters) |
//! | `lod`       | `.lod` descriptors mapping LOD levels to models      |
//! | `terrain`   | global height map and the 64×64 streamed terrain tiles |
//! | `dds`       | DDS container + CPU decoders (DXT1/5, A4R4G4B4, A8L8) |
//! | `landscape` | terrain materials: gameplay table, meta materials, slope slots, atlases |
//! | `placement` | object placements (`.blo`) with absolute transforms    |
//! | `areaset`   | which shared model container (`areasetNN.blz`) a tile uses |
//! | `vehicle`   | `.mvdoll` vehicle dolls: part tree, wheels, suspension, engine |
//! | `havok`     | Havok 5.5 packfile container (sections, fixups, class names)   |
//! | `collision` | rigid bodies and shapes out of `.pfx` packfiles                |
//! | `cgd`       | compiled static geometry lists of the location containers     |
//! | `vegetation`| the procedural vegetation generator (types, sets, noise, rand) |
//! | `grass`     | grass types and the 8 m-patch grass generator                |
//! | `impostors` | tree impostors: atlas rectangles of the distant trees        |
//! | `weapons`   | the weapon table, upgrades, ammunition, grenades           |
//! | `enemies`   | enemy soldiers: classes, weapons, health, dolls per stage  |
//! | `pickups`   | weapon pickups and stashes (exported entities)             |
//! | `heat`      | heat levels and the reinforcements per faction and band     |
//! | `chaos`     | activity types (chaos, money per sabotage), the chaos stages |
//! | `blackmarket`| the Black Market's items, prices, unlocks and upgrade costs |
//! | `missions`  | the mission table, rewards, rules, scripts, strongholds and colonels |
//! | `civilians` | world simulation (people, cars per zone), civilian roam areas |
//! | `roads`     | the AI road graph (`default.rg`) and the road splines        |
//! | `text`      | the localized text database (`ms15text.csv`): key → string per language |
//! | `weather`   | sun/moon orbit, sky colour gradient, cloud settings           |
//! | `fsb`       | FMOD FSB4 sample banks (MPEG samples → plain MP3)            |
//! | `fev`       | FMOD Designer event projects: events, layers, sound definitions |
//! | `soundmap`  | the game's sound ids (bank, sound) → events: weapons, characters, vehicles, zones, music |
//! | `font`      | the HUD's bitmap fonts (`fonts.blz`): DXT5 atlas, glyph table, metrics |
//! | `browse`    | the asset browser's model: file index, folder tree, search, previews |
//!
//! Formats still to do, roughly in the order a renderer needs them:
//! the terrain tiles' zone-map and mesh sections, `.pfi` collision instances, ADF (" FDA",
//! particle/effect definitions), `*.aitile` navigation tiles, Anark UI, Havok
//! packfiles (collision, animation, behaviour).

pub mod animation;
pub mod areaset;
pub mod blackmarket;
pub mod browse;
pub mod camera;
pub mod cgd;
pub mod chaos;
pub mod character;
pub mod civilians;
pub mod collision;
pub mod compress;
pub mod dds;
pub mod destruction;
pub mod detect;
pub mod enemies;
pub mod error;
pub mod event_track;
pub mod fev;
pub mod foliage;
pub mod font;
pub mod fsb;
pub mod grass;
pub mod hash;
pub mod havok;
pub mod heat;
pub mod icons;
pub mod impostors;
pub mod landscape;
pub mod location;
pub mod lod;
pub mod missions;
pub mod names;
pub mod particle;
pub mod pickups;
pub mod placement;
pub mod player;
pub mod property;
pub mod ragdoll;
pub mod rbm;
pub mod reader;
pub mod roads;
pub mod sarc;
pub mod soundmap;
pub mod tab;
pub mod terrain;
pub mod text;
pub mod texture_policy;
pub mod types;
pub mod vegetation;
pub mod veglod;
pub mod vehicle;
pub mod vfs;
pub mod weapon_mounts;
pub mod weapons;
pub mod weather;

pub use error::{Error, Result};
pub use vfs::GameFs;
