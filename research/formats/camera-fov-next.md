# Gameplay camera FOV (JC2-141, closes Gap 1 of JC2-126, replaces the proposal JC2-098)

*9 October 2026, `claude/camera-fov`. Retail `JustCause2.exe` sha256 `df99a479...4f80`. Addresses are native VAs; the
decompiler export is the ignored local `research/analysis/pseudo/_unnamed`. Probe:
`python -B research/tools/camera_fov_probe.py $game --export $export --tool $jc2tool` (exit 0 only if every check below passes).*

## Answer

* The default on-foot FOV is **40 degrees, vertical** (0.6981317 rad, f32 `0.6981316804885864`): camera setting id 0
  "Onfoot Camera", `settings/camera_settings.bin`, key `Fov` (hash `0x4995a552`) = 40.
  (`m[11] = -1`), aspect = back-buffer width / height. The horizontal FOV follows the window: 65.81 deg at 16:9, 51.77 deg at 4:3.
* The 65 deg (1.1344640 rad, `0x3f91361e`) that JC2-098 and `terrain_material.rs` call "the game camera default" is only the
  value the graphics-engine camera object is constructed with (`FUN_00924d80`). The camera manager overwrites it on its first
  frame. It is never a gameplay FOV. Bevy's 45 deg coincides with the *struct default* a camera setting gets when no key sets one (`0x42340000` = 45.0,
  `FUN_005852e0`), which Parachute, Freefall, swim, dive, ragdoll, ladder, death and the helicopter/plane/reeling base cameras use;
  it is not the on-foot value.
* The realised FOV moves with the mode (aim 20, sniper 15 -> 3, vehicles 50 -> up to 90 with speed, see tables), so every
  FOV-scaled distance is mode dependent. Terrain LOD radii scale with `1/fov^2` and vegetation fade distances with
  `max(fov^2, 0.38)` (**no upper cap**, correcting the old "f = 1 for FOV >= 1 rad" fixture note, see Corrections).

## Proven

### 1. The camera object and its writer (`0092c620`)
   sine/cosine from the constants `0x0108c5a4/a8/ac` = 2pi, pi/2, 1/pi) and sets the dirty byte `camera+4`.
2. The only callers (the probe scans the whole export): `0067c330` (gameplay, camera manager, once per frame), `00a79d80` (copies
   the main camera's FOV/near/far into the water reflection camera each frame), `00980ef0` (render-side camera set-up from
   `00970000`, identity view, not gameplay), and the cutscene writers `008fd280`, `008ffdd0`, `00903ef0` (cutscene key
   FOV in degrees times `0.017453292`; `008ffdd0` restores the saved value from the track object at `+0x19c`).
   is copied from `DAT_011919f4+0x67c` by `0092a830`, and `00968700` writes that field as `width / height` of the new back buffer. Hence a fixed vertical FOV and a window-dependent horizontal FOV.
4. The graphics camera constructor `FUN_00924d80` initialises `+0x618 = 0x3f91361e` (65 deg), `+0x634 = 1.0`, `+0x624 = 0.5`,
   `+0x628 = 50000.0` (`0x47435000`). Near/far used in play are set later (`00980ef0` / camera manager); not decoded here (Gap 7).

### 2. The gameplay chain
`CCameraControlManager` (singleton `DAT_011900f8`, ctor `00762eb0`) holds three `CCameraController` states of 0xa8 bytes at
`+0x18c`; the active one is index `+0x10`. The FOV of a state is at state `+0x70` (manager `+0x1fc`).

* **Per-frame application, `0067c330` (called from the manager update `0081c960`, before the matrix blend `0062be20`).**
  `target = state[active].fov (+0x1fc)`, `now = camera+0x618`. If no transition has ever been started (`manager+0x20 == 0`) the target
  is written directly. Otherwise `CCameraTransition::update` (`0053efe0`) runs on the transition object at `manager+0xf4` and
  `fov = now + w * (target - now)` is written through `0092c620`, `w` = the transition weight at `manager+0x104`
  transition `+8`, property key `0xaa773667`) while the active byte (`+0x21`) is set, an optional start delay (`+0x1c`) has elapsed and
  the mode (`+0x28`, key `0x77fd4815`) is 1; mode 0 jumps `w` to 1. The weight multiplies the remaining difference **each frame**
  (it is not an interpolation parameter between a start and an end value), so the visible curve depends on the frame rate.
* **State builder, `00838720` (from `SetCamera` `00548f30`, which `SetCameraById` `00600850`, vtable slot 3, calls).**
  `state.fov = min(def.fov * fovfactor, 1.4)` with `def.fov` at camera-definition `+0xc0` (radians), `fovfactor = _DAT_010655a0`
  (initial `1.0`, retail data byte `0x3f800000`; command line `/fovfactor=f`, only consumer is this line), and the cap 1.4 rad
  (80.21 deg). The same function creates the filters (`def+0xfc` selects through the factory `005d9760`).
* **Camera definitions, `007d44b0` (called with the three file names by `00610000`).** Each `settings/camera_settings.bin` entry is read
  by `00808070` into a 0x178-byte record initialised by `005852e0` (default `+0xc0 = 0x42340000` = **45.0**). `00808070` first reads the
  entry's type string (key `0x8cc2f0ff`, e.g. "Car") and, if found, recurses into that section of `camera_defaults.bin`
  (so type defaults are applied first), then reads the entry's own keys, so an entry's `Fov` overrides the type's. `007d44b0` converts
  the angle fields (Fov, `+0xcc..+0xd8`, `+0x124/+0x128`, `+0x14c`, `+0x16c`) from degrees with `* 0.017453292`. Settings ids not present
  get a disabled record (`+0xe0 = 1`).
* **Which setting is active** is `controller+4` (vtable slot 7 `009573c0`, read at start-up by `005f0000` after `COnFootController`
  is fetched; initial -1 from `0090f300`). `006f77a0` (called right after the player settings are loaded, `00660000`) fills it from
  `settings/player.bin` -> `player` -> **`cameras`** (hash `0x38a7a205`; 16 keys, all matched against the key table at exe
  `0x00ec2150`): OnFoot 0, Ladder 0, Parachute 1, SkyDive 2, BaseJump 2, Swim 38, Dive 39, Ragdoll 25, ReeledIn 30, ReeledHang 65 and
  raw-hash keys 26, 3, 5, 0 (aim table), 69 (crouch), 0 (unmounted gun). Vehicle controllers get theirs from the vehicle's own
  property block (`camera_id` key `0x742cdd23`, `camera_id_reverse` `0xb4ad6a59`, see tables).
* **On-foot choice per frame** (`COnFootController` update, `text_00820000.c`): crouch states -> `DAT_01076acc` (69, "Crouch
  Camera", 40 deg); special state set -> `controller+0x310`; otherwise `DAT_011fccec` (0, "Onfoot Camera", 40 deg).
  **Aim cameras:** `DAT_011fcccc[aim mode]` (slot 8, `004f9f20`; `[0] = -1`, then 26, 3, 5, 0 written by `006f77a0`).

### 3. Filters that change the state FOV (`state+0x70`)
* **Vehicle speed filter** (factory case 2, `00699c70` build, `0068d720` per-frame): speed = |velocity| of the player character
  (`FUN_008a0bd0`). With `[smin, smax]` = key `0xf8ae9bfd`, base = `Fov`, peak = key `0xcc2187d9`: for `speed <= smin` the FOV springs back
  to the base; otherwise `t = clamp((speed - smin)/(smax - smin))`, target = `base + (peak - base) * t` and the state FOV follows it with
  a critically parametrised spring `v += (-k x - c v) dt'; x += v dt'` (`dt' = min(dt, 0.1)`, dead band 1e-5, k and c = key
  `0xa3b42ba2`). The filter also moves the camera back by the `cot` difference ("reverse dolly"), keeping subject size.
  Data (see table): Car 50 -> 65 over 10..40 m/s, Motorcycle 50 -> 60, Boat 50 -> 75 over 5..60, Plane 45 -> 85 over 10..80,
  Chopper 45 -> 75 over 10..50; every setting with filter type (`0xfb71b57e`) 2 and no "Reverse" suffix.
* Filter type values present in retail data: absent, 0 (reverse cameras, none), 2 (speed), 4 (water, "Swimming"). Type 1 (a combat FOV
  spring between `Fov` and `0xcc2187d9`, `007adb00`/`0052cd00`) and 3 (parachute) exist in the factory but no retail setting selects them.
* **Sniper zoom**: `settings/player.bin` -> `player` -> section `0xa2712e04` (read by `00660000`): min FOV 3 deg (`0x9f2c04d4`, scaled by
  0.017453292 into `player+0x1088`), max FOV 15 deg (`0x94e95f3c`, `+0x1084`), rate 0.4 (`0x3631a212`, `+0x108c`, rad/s), HUD scalars
  add/subtract `rate * dt` to the **active state FOV** (`FUN_00793c10` writes `state+0x70`), clamped to [3, 15] deg;
  `GetSniperZoom` (`006e6260`, Lua) reports `(fov - 15 deg)/(3 deg - 15 deg)` for player mode 2 or 3.

### 4. Data fixtures (probe-asserted; `fov_rad` = f32(deg * 0.017453292))
Source of the value: *set* = the entry's own `Fov`, *type* = `camera_defaults.bin` section of its type, *default* = struct default 45.

| id | setting | type | FOV deg | rad (f32) | source |
|---|---|---|---|---|---|
| 0 | Onfoot Camera | Normal | **40** | 0.69813168 | set |
| 69 | Crouch Camera | Normal | 40 | 0.69813168 | set |
| 26 | Shoulder cam (aim idx 1) | Normal | 20 | 0.34906584 | set |
| 3 | Sniper1 (aim idx 2) | Normal | 15 | 0.26179940 | set |
| 5 | Sniper on roof (aim idx 3) | Normal | 15 | 0.26179940 | set |
| 67 | Roof shoulder cam | Normal | 25 | 0.43633229 | set |
| 1 | Parachute Camera | - | 45 | 0.78539819 | default |
| 2 | Freefall Camera (also BaseJump) | - | 45 | 0.78539819 | default |
| 38 / 39 | Swimming / Diving | - | 45 | 0.78539819 | default (38 has the water filter) |
| 25 | Ragdoll | - | 45 | 0.78539819 | default |
| 40 | Ladder | - | 45 | 0.78539819 | default |
| 27 / 28 | Mounted Minigun / Machinegun | - | 40 | 0.69813168 | set |
| 9 / 21 / 30 / 45 / 65 | Reeled-in jump, Hang drop, Reeled-in wall, heli hang, hang | - | 50 | 0.87266463 | set |
| 24 / 29 / 31 / 42 / 43 / 90 | Reeling cameras | Reeling | 45 | 0.78539819 | type |
| 4 (Sedan), 62 (Jeep), 36 (Truck), 64 (Sports), 70 (Bus), 32 (Tank), 63 (Micros) ... | cars | Car | 50 -> 65 | 0.87266463 -> 1.13446403 | type + speed filter |
| 12 ATV, 37 Motorcycle | bikes | Motorcycle | 50 -> 60 | | type + speed filter |
| 33 SmallBoat, 35 SpeedBoat, 52, 61, 83 | boats | Boat | 50 -> 75 | | type + speed filter |
| 44, 59, 66, 85, 86 | helicopters | Chopper | 45 -> 75 | | default + speed filter |
| 34, 49, 57, 60, 84 | planes | Plane | 45 -> 85 (60 for id 60) | | default + speed filter |
| 79 / 87 / 89 | plane reverse cameras | Plane | 65 / 55 / 65 | | set, no filter |
| 71..78, 80, 81 | other reverse cameras | Car/Motorcycle/Boat | 50 | | type, filter type 0 |

Per-vehicle camera ids (scratch census of the 78 vehicle `.ee` files that carry an `.mvdoll`, driver controller; not asserted by the probe): sedans and
most cars 4 (`camera_id_reverse` 72); jeeps/military 62 (74); trucks/buses 36 (75); Sports 64 (73); Micros 63 (72); tank/APC 32 (76);
bus coach 70 (71); ATVs/tuk-tuks 12 or 62/63; motorbikes 37 (77); boats 33, 35, 52, 61 (80/81); small helicopters 66, civil/military
44, lifter 59 (reverse 79); planes 34, 49, 60, 84 (reverse 79 / 87 / 89). Eleven vehicles have no controller block in their own
`.ee` (Gap 4). Key `0x742cdd23` is `camera_id`, read for every controller by `008a1a60`.

### 5. Transition fixtures (`settings/camera_transitions.bin`, 32 groups)
Per group: position blend `0xe380ec1d` (+4), FOV weight duration `0xaa773667` (+8), mode `0x77fd4815`; group `Mapping` lists (From, To) setting ids.
The manager's default transition (no mapping found) is 0.5 / 0.5 (`00762eb0`: `[0x18] = [0x19] = 0.5`).

| group | pos | FOV w | mode | pairs |
|---|---|---|---|---|
| closeup_aim | 0.3 | **0.4** | 1 | 0<->26, 0<->67, 26<->67 |
| Sniper | 0.1 | **0.1** | 1 | 0<->3, 26<->3, 3<->67 |
| Crouch | 0.2 | 0.2 | 1 | 0<->69 |
| freefall | 0.5 | 0.5 | 1 | 0->2, 1<->2 |
| swimming / diving | 0.5 | 0.5 | 1 | 0, 38, 39 combinations |
| mounted | 0.2 | 0.3 | 1 | 0<->27 |
| ragdoll | 0.5 | 0.5 | 1 | 25->0, 38->25, 39->25 |
| reverse_cam | 0 | 0 | 0 | 44 car/bike/boat pairs (instant) |
| Hard_Cuts | 0.1 | 0.5 | 1 | 41 pairs |

(The 32 groups, with all mapping pairs, are reproduced by the probe; the other groups are vehicle enter/exit, cling, reel and kill-cam cuts.)

### 6. Consumers of the realised FOV (`camera+0x618`)
* Terrain LOD `k = min(1/fov^2, 999.99994)` (`00a61220`, `text_00a60000.c`): at 40 deg `k = 2.0517542`, LOD0 radius `25 k = 51.29 m`,
  LOD1 `250 k = 512.9 m`; at 15 deg 364.8 m / 3647.6 m; at 65 deg 19.42 m / 194.2 m.
* Vegetation `f = max(fov^2, 0.38)` (`00a83740`, `00a843a0`; render context `+0xca8` = `camera+0x618`, copied by `00950000`): 40 deg
  0.48738784; 45 deg 0.61685032; 50 deg 0.76154357; 65 deg 1.28700864; 90 deg 2.46740127; any FOV <= 35.3 deg (0.6164 rad) floors at 0.38
  (aim 20 / sniper 15 / 3 deg: 0.38). There is no `min(.., 1)`.

## Inferred

1. The graphics-camera value at the end of a frame is the state FOV one frame late for filter-driven changes (`0081c960` calls
   `0067c330` before `0062be20`, which runs the filters). Not run-verified.
2. Because `w` multiplies the remaining difference every frame, the observed blend is frame-rate dependent; at 60 Hz with a 0.4 s ramp half of
   the change is done after about 5 frames and the rest within about 10, i.e. effectively a short ease-out rather than a 0.4 s linear fade.
   A port that keeps the per-frame formula and real `dt` reproduces the native curve at any rate by construction.
3. **Freeze quirk**: `0053efe0` only writes `w` inside its ramp branch; if the FOV difference is 0 on the first frame of a transition it
   clears the active byte and leaves `w` at 0 (set by the start routine `006c7500`), and `0067c330` reads the stored `w` (the return
   value is discarded, bytes at `0x67c3a8`). Speed-filter FOV changes after such a transition (e.g. Sedan -> Sedan Reverse, equal base FOV) would
   then not reach the camera until the next transition starts. Decoded from the code only.
4. Aim index meaning: the aim table is [none, Shoulder 26, Sniper1 3, Sniper-on-roof 5, 0]; the zoom keys act in aim mode 3 while the HUD
   zoom reads modes 2 and 3, so Sniper1 and Sniper-on-roof are probably modes 2 and 3 and index 1 the ordinary rifle shoulder aim. The
   mode-to-weapon mapping was not traced.
5. Cutscene FOV key values are degrees (`008fd280`, `00903ef0`; the latter keeps values below 1.0 as radians); the cutscene data format
   was not decoded.

## Corrections to existing research and code
* `tree-impostors-next.md` "f = 1 for any FOV >= 1 rad" and the table fixtures with `f = 1`: native `f = max(fov^2, 0.38)` has no
  native on-foot 40 deg `f = 0.487388`, the lodfactor 0 foliage fade runs 112.8 m (full) -> 167.8 m (gone) instead of 55 -> 110 m, and the Top/Trunk
  card starts move out by the same factor 2.05. The document's `f = 1` rows are valid only for a FOV of exactly 1 rad (57.3 deg).
* `terrain.md` / `terrain_material.rs`: `NATIVE_FOV = 1.134464` is the graphics-engine constructor default, not a gameplay value. The terrain
  scale must use the realised camera FOV (40 deg on foot -> LOD0 51.3 m / LOD1 512.9 m, not 19.4 / 194.2 m).
* JC2-098 text "camera constructor sets 65 deg, gameplay uses it": see Answer.
* The project camera (`world.rs`: `PerspectiveProjection { far: 30_000.0, ..default() }`, Bevy default `fov = pi/4`, `near = 0.1`) is 45 deg
  vertical: 5 deg too wide on foot and wrong in every other mode.

## Gaps (each blocks parity; none closed by guessing)
1. **Aim-mode meaning.** Which weapon/state selects `DAT_011fcccc[1..4]` (26, 3, 5, 0) and what player mode value `+0x300` / `FUN_00653d90` each is. Blocks
   which camera (20 vs 15 deg) a given weapon aim uses. Look at the aim-state writers around `005c0000` and `FUN_00653d90`.
2. **Special on-foot states.** `controller+0x310` overrides and the state-hash tests in `FUN_00572ca0` / `FUN_005b21d0` / `FUN_0059be20`
   (crouch, cover, roof) are named only by hashes; the camera ids reached through `+0x310` (e.g. 67 "Roof shoulder cam", 6/10/11/13 stunt cameras,
   45/65 hang) are not traced. Blocks the exact crouch/cover/roof FOV set.
3. **Reverse-camera condition and camera_id_reverse use.** Which controller flag (`+0x2c/+0x30/+0x34` families) selects `camera_id_reverse`.
4. **Vehicle ids**: 11 vehicles lack an `.mvdoll` in their own `.ee` (e.g. `arve.v067_speedplane`, `arve.v076_jet_blackplane`, `lave.v007_military`,
   `seve.v087_catamaran_speedboat`); where their `camera_id` lives is unknown. Passenger/cling/stunt camera ids (4, 7, 8, 44, 65, 6...) are listed only as census data.
5. **Parachute / grapple.** Parachute (1) and Freefall (2) have no FOV key (45 deg default) and no filter; whether `ParachuteParams.bin` or the
   parachute controller changes FOV elsewhere was not traced. Grapple transitions (`Reeling`, id 24/29/31/42/43/90 = 45 deg; hang 65 = 50)
   and the grapple-hook aim camera (`grapplingparams.bin` `camera_id = -1`) were not followed through the grapple controller.
6. **Other filters**: the position/offset/pitch/run/collision filter families (`def+0x104/0x108/0xbc/0x10c/0x100`) were not audited for
   writes to `state+0x70`; the speed filter and the sniper zoom are the only proven FOV writers besides `00838720`.
7. **Near/far planes** in play (graphics ctor 0.5 / 50000; other notes cite 0.1 / 40000) and the camera manager's initial FOV before the first
   state build (`manager` ctor value 0x3f5f66f3 = 50 deg in each controller) were not resolved.
8. **Cutscene FOV** (`CCutsceneTrackCamera/SimpleCamera` key format) and the `0xa2712e04` HUD scalars (`+0x1090/+0x1094`, probably look-speed
   scaling by zoom) are undecoded.
9. **In-game verification.** No runtime capture exists of the realised FOV curve, the freeze quirk (Inferred 3) or the 1-frame lag; all of it is static analysis.

## Bounded Rust handoff
1. `jc2-formats`: add a `camera_settings` reader (`settings/camera_settings.bin`, `camera_defaults.bin`, `camera_transitions.bin`, `player.bin` `cameras` and
   `0xa2712e04`) using the keys in the tables; effective FOV = own `Fov` -> type default -> 45; degrees to radians as `f32(deg * 0.017453292)`; `min(.., 1.4)` after `fovfactor`.
2. `jc2-game`: replace the Bevy default projection of the gameplay camera with `fov_y = state FOV` (on foot 40 deg = `0.6981317`), keep aspect = window
   width / height, near/far unchanged until Gap 7; leave the model-viewer / capture camera alone (JC2-088).
3. Implement the per-frame application exactly (`fov += w * (target - fov)`, `w` ramped by `dt / duration` from the transition table, default 0.5 s, mode 0 = instant) and the speed
   filter (`k`, `c`, dead band 1e-5, `dt' = min(dt, 0.1)`) as a separate step after the first slice.
4. Feed the realised FOV (not a constant) into `terrain_material::lod_scale` and the vegetation `f`; rename `NATIVE_FOV` (65 deg) to the engine-constructor default.
5. Tests: the probe's `settings` / `terrain_lod` / `vegetation_f` / `projection` fixtures (on-foot 40 deg: k 2.0517542, f 0.48738784, 65.81 deg horizontal at 16:9).
   Slice 1 (static per-mode FOV by camera id) is safe to land alone; aim/sniper selection waits on Gap 1.
