# Weapons (`settings\weapons.bin`)

A property container; every weapon is a `weapon_N` node under the root with an `id`
(the weapon class, shared by variants: 128 is every vehicle-mounted light machine gun)
and a `name`. Thrown weapons (`CHandGrenade` …) sit in a separate `#a17fc6ed` list with
string ids (`WEA33`) and per-upgrade-level sub-nodes (`level_0` … `level_5`).
`jc2_formats::weapons::parse` returns every node that has both an `id` and a `name`
(49 in the retail data).

## Keys

Named ones were found by hashing candidates (Jenkins lookup3, `jc2tool hash`) against
the Beretta's 80 keys; the rest are inferred from how the value varies over the table.

| key | name | Beretta | notes |
|---|---|---|---|
| `id` | | 2 | weapon class |
| `name` | | `Beretta Handgun` | `#94142e13` is the short name (`Beretta`) |
| `model` | | `Weapons\WEA02\WEA02_LOD1-WEA02.rbm` | in `general.blz`, lower-cased |
| `#be0ec6e0` | *(damage)* | 19 | shotgun pellet 13, Assault 32, Revolver 54, Sniper 300, RocketLauncher 525, vehicle rockets 1000 |
| `ammo_max_range` | ✓ | 100 | m |
| `ammo_velocity` | ✓ | 750 | m/s; sniper 2000, grenades/rockets 60–100 |
| `#58b8bb06` | *(pellets)* | 1 | 10 on the pump shotgun |
| `#84ecc4af` | *(impact force)* | 1 | 0.01 … 400 (sniper) |
| `weapon_accuracy` | ✓ | 1 | |
| `max_bullet_spread` | ✓ | 0 | |
| `#e56e25f4` | *(projectile kind)* | 10 | 10 bullets, 4 rockets, 61 grenades, 246 vehicle rockets |
| `sound_fire_id` | ✓ | 6 | |
| `tracer_fire_velocity` | ✓ | 100 | m/s; MGs 200 |
| `tracer_scale_factor` | ✓ | 0.3 | MGs 0.5 |
| `aim_type` | ✓ | — | 1 on vehicle cannons |
| `#62d2f184` | | `WEAPON_MODIFIER_DUALWIELD_PISTOL` | which hand-modifier set |
| `#94142e13` | *(short name)* | `Beretta` | the HUD shows it (`Scorpion SMG`, `MORETTI CCW` on the pump shotgun) |
| `reloadtime` | ✓ | 0.8 | s; 1.1 SMG, rifles, launchers; 1.2 pump shotgun |
| `weapontype` | ✓ | 2 | equal to `id` on the hand guns |
| `bulletspershot` | ✓ | 1 | the `#58b8bb06` above: 10 on both shotguns |
| `unlimited_ammo` | ✓ | 0 | 1 on the minigun |
| `burstrate` | ✓ | — | 0.04 on the rifles and launchers |
| `clipmodel`, `weaponcount` | ✓ | | the magazine's model (SMG, Assault); `weaponcount = 49` in the root `weapons` node |
| `#a0d58b16` | *(ammo type)* | 1 | SMG 2, MG 3, one-hand shotgun 4, sniper 5, GL 6, both rocket launchers 7, Assault 8, minigun 12, revolver 14, pump 15 — indexes `ammunition.bin`'s `max_ammo_N` (assumed from the values: shared by the two rocket launchers, 0 → `max_ammo_0 = 0`) |
| `#01dbd94a` | *(automatic)* | 0 | 1 on the SMG, Assault, MG, minigun (and the Panay launcher) — taken as "fires while held" (assumed) |
| `#7619481b` | | 0.11 | authored V.y, transformed into weapon emission frame; see `weapon-sockets-next.md` |

### `#9acc62e9`: how the AI uses it

Tagged with an `AI_MODIFIER_*` string (`#4fdb09b7 = "AI_MODIFIER_PISTOL"`).

| key | Beretta | Assault | Sniper | notes |
|---|---|---|---|---|
| `min_range` | 0 | 0 | 0 | m |
| `max_range` | 50 | 50 | 100 | m; Flak 100–1000 |
| `min_burst_delay` | 0.15 | 0.5 | 2 | s between bursts |
| `max_burst_delay` | 0.3 | 1.5 | 2 | |
| `magazine_size` | 8 | — | — | hand guns only (SMG 32, Revolver 6) |
| `min_burst_length` | 0 | 1 | 0 | s; MG 2 |
| `max_burst_length` | 0.1 | 2 | 0 | s; MG 4 — a pistol "burst" is a shot or two |
| `firerate` | 0.05 | — | — | s between shots, hand guns only |

## Upgrades (`settings\weapon_upgrades.bin`)

Under the root `#2640d65f`, one node per weapon keyed by the **hash of its `weapons.bin`
name** (`Beretta Handgun` → `#c4e68c40`), each key a `level_0` … `level_5` node (the six
upgrade levels). All keys named by hashing: `firerate` (s between shots), `damage`,
`damage_max`, `impulse`, `magazinesize`; two unnamed (`#6a1fb54c` 2.2 → 1 on the Beretta,
falls with each level: base aim-scatter degrees; `#52aa95ac` 0.07: per-shot character
accuracy decrement). Native consumer evidence and limits are in
`weapon-sockets-next.md`; neither field is implemented by `jc2_formats::weapons::Upgrades`.

| weapon (level 0 → 5) | `firerate` | `damage` | `magazinesize` |
|---|---|---|---|
| Beretta Handgun | 0.12 → 0.1 | 16 → 28 | 12 → 22 |
| Scorpion_SMG | 0.0667 → 0.0542 | 16 → 28 | 30 → 40 |
| Shotgun (pump action) | 0.325 → 0.2 | 12.5 → 22 | 6 → 16 |
| Assault | 0.1091 → 0.0941 | 29 → 51 | 20 → 30 |
| RocketLauncher | 0.8 → 0.3 | 750 → 1000 | 3 → 8 |
| Revolver | 1 → 0.75 | 65 → 115 | 7 → 12 |
| MachineGun | 0.13 → 0.0985 | 35 → 62 | 26 → 41 |
| Sniper | 1.5 → 1 | 125 → 220 | 4 → 9 |
| Onehand Shotgun | 0.4 → 0.3 | 12.5 → 22 | 3 → 8 |
| GrenadeLauncher | 0.325 → 0.225 | 225 → 350 | 5 → 10 |

Vehicle guns carry `magazinesize` 1000 (or 0), i.e. no magazine.

**Upgrade levels in jc2-rs.** Rico's guns are at the level the Black Market bought them
(`blackmarket.bin`'s `costs`: 5, 10, 15, 20, 25 weapon parts for the steps 0→1 … 4→5;
`chaos.md`). A gun's damage, fire interval and magazine are the level's `damage`,
`firerate` and `magazinesize` (`Arsenal::set_level`: the magazine keeps its rounds and
grows from the reserve; the reserve is re-sized to `max_ammo_N` magazines of the new size);
the HUD lights one star for the gun and one per level. The Beretta: level 0 16 damage, 12
rounds, 0.12 s; level 1 18, 14, 0.116 s; level 5 28, 22, 0.1 s. The soldiers keep the
`weapons.bin` damage (`#be0ec6e0`), which is taken as the AI's.

## Ammunition (`settings\ammunition.bin`)

`ammunition { max_ammo_0 … max_ammo_19 }` (named by hashing; `max_ammo_` is an exe
string) and four sound blocks (`whiz_sounds`, `ricochet_sounds`, … with `soundbank_id`,
`sound_id`, `max_sounds`, `min_dt_deviation`, `var_id_focus`). `max_ammo_N` for the
ammo types above: Beretta 15, SMG 7, MG 7, one-hand shotgun 10, sniper 4, GL 6, rockets 4,
Assault 9, minigun 20, revolver 6, pump 9. These only make sense **counted in
magazines** (4 rockets in total would be too few; 4 × 3 = 12), so jc2-rs carries
`max_ammo_N × magazinesize` rounds in reserve (assumed): Beretta 12/180, SMG 30/210, pump
6/54, Assault 20/180, rockets 3/12. `jc2_formats::weapons::ammunition`.

Grenades: the thrown list's `#7b655f19` (10 at level 0 → 20 at level 5) is taken as how
many he carries (assumed). `player.bin` has `grenade_count { 0 = -1, 1 = -1, 2 = 0 }`
(under `#ec970be2`, next to the `weapon_pickup.bin` template and `Grapplinghook`) — the
starting counts per thrown item, -1 maybe "as many as fit"; not used.

Still unresolved: exact emission-to-ballistic-origin chain, attachment slot/hand
ownership and thrown weapon upgrade levels (the grenade stays at level 0).
The traced angular/accuracy upgrades are not applied by the current runtime.

## Grenades, charges, flares (`#a17fc6ed`)

String ids, a `_class`, and the explosion values per upgrade level in sub-nodes
(`level_0` … `level_5`, all named keys found by hashing):

| key | WEA33 grenade | WEA35 triggered charge | notes |
|---|---|---|---|
| `_class` | `CHandGrenade` | `CTriggeredExplosive` | also `CFlare`, `CLUSTERBOMB` |
| `model_name` | `Weapons\WEA33\WEA33_LOD1-WEA33.rbm` | `…WEA35_LOD1-TTE.rbm` | |
| `collision_name` | `…WEA33_LOD1-WEA33_COL.pfx` | | |
| `damage` | 500 … 750 | 750 | per level |
| `impulse` | 1000 … 1500 | 1500 | |
| `range` | 5 … 6.25 | 6.25 | m, the blast radius (the flare has a plain `radius = 3`) |
| `detonation_time` | 3.5 | 1 | s, the fuse |
| `detonation_effect_id` | 64 | 20 | |
| `speed` / `max_speed` | 40 / 25 | — / 25 | m/s, the throw |
| `#7b655f19` | 10 … 20 | 10 | per level: taken as how many he carries (assumed) |

`jc2_formats::weapons::throwables` reads them (level 0).

## Vehicle and character types

`settings\vehicletypes.bin`: 91 nodes, each `entity` (`lave\lave.v014_jeep.ee`),
`mem_cost`, `perf_cost` (2 everywhere) and `#c097e6ef`, which runs 310 (Vespa), ~600
(bikes), ~2000 (cars: the jeep 1980), 4000 (helicopters), 5000–6500 (buses, the fire
truck, planes) — jc2-rs takes it as the vehicle's health. `charactertypes.bin` lists 103
character types by `name` (`pd_gov_base01`) with only `mem_cost` (100) and `perf_cost`.
`jc2_formats::types` reads both.

## Health

`player.bin` names `health = 400` for Rico (in the `#5b831e1c` block). Soldiers' health
and weapons are in `enemytypespawnsettings.bin` (`spawning.md`): grunts 100 (120 at the
last stage), elites 250 (265).

## How the game uses it (jc2-rs)

`combat.rs`: Rico starts with the Beretta, the Scorpion SMG, the pump shotgun (10 pellets
in a wide cone), the assault rifle and the rocket launcher (`1`–`5`), each firing every
`firerate` s (upgrade level 0) and automatic when `#01dbd94a` says so; guns picked up
join them on `6`–`9`. Their models go in `AttachHandRight`. Every gun has a magazine
(`magazinesize`) and a reserve (`max_ammo_N` magazines) (`ammo.rs`, `WeaponState`):
firing spends a round, an empty magazine reloads by itself and `R` reloads early, for
`reloadtime` seconds during which nothing fires; switching guns drops a reload under
way. Grenades are counted (10). The HUD shows `Beretta 12/180`, the slot, the grenades.
Rico's damage is the upgrade table's at the gun's level (16 on the Beretta at level 0; it
was the `weapons.bin` 19 before the upgrade levels came in).
A gun whose `#e56e25f4` is 4 (rocket) or 61 (lobbed) fires a projectile at `ammo_velocity`
that bursts on contact with its damage over the grenade's 5 m; the others cast a ray from the camera along its aim out to `ammo_max_range` plus the camera's distance, with a
scatter from `weapon_accuracy`, damages anything with `Health` by the damage value and
pushes dynamic bodies by `force`. Soldiers carry the weapon their class rolled
(`spawning.md`): within its AI `max_range` and in line of sight they stop their rounds,
turn to Rico and fire bursts of `min…max_burst_length` seconds at the AI `firerate` (or
the upgrade `firerate`), `min…max_burst_delay` apart; the AI `magazine_size` (else the
upgrade one) runs out and they pause for `reloadtime`. Every shot (every pellet) rolls
to hit, more likely up close, the chance spread so a burst lands about two hits (tuned).
The dead drop their gun (`spawning.md`), lose their collider and are cleared after 30 s;
Rico at zero lies a few seconds and gets up at the start.

`explosion.rs`: `G` throws the grenade (a small dynamic body, a lob at 0.7 × `max_speed`)
that goes off after `detonation_time`. A blast damages everything with health by
`damage × (1 − d / range)` (vehicles measured to roughly their skin) and pushes dynamic
bodies within 1.5 × `range` (`impulse` scaled ×8 — tuned so a grenade throws a car), with
a fireball and a flash. A vehicle at zero health bursts (9 m, 450 damage — its
explosion effect is not decoded), throws its driver out, chars black and smokes;
helicopters drop.
