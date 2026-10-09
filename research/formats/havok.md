# Havok packfiles and collision

Implemented in `crates/jc2-formats/src/havok.rs` (container) and `collision.rs` (shapes,
tile instance lists); `jc2tool collision <game> 31x14.nlz` summarises every `.pfx` in a
container, `jc2tool collision <game> 31x14.nlz/gb010_lod1-a_col.pfx` prints one shape tree.

The game is built on Havok 5.5.0 (physics, animation, behaviour). Everything Havok ships
as a **binary packfile** (`Havok-5.5.0-r1`, 32-bit, little-endian), usually behind a
12–16 byte wrapper: `"PFX\0"` for a prop's collision, `"PFS\0"` for a tile's.

## Packfile container

```
u32 0x57E0E057, u32 0x10C0C010      magic
i32 userTag, i32 fileVersion (5)
u8 layout[4] = 04 01 00 01          4-byte pointers, little-endian, reuse padding, empty-base-class opt.
i32 numSections (3), i32 contentsSectionIndex (2), i32 contentsSectionOffset
i32 classNameSectionIndex (0), i32 classNameSectionOffset
char version[16] = "Havok-5.5.0-r1", i32 flags, i32 pad
48-byte section headers: char tag[19] + NUL, i32 absoluteDataStart, then offsets *within
  the section* of: localFixups, globalFixups, virtualFixups, exports, imports, end
```

Sections: `__classnames__` (`u32 signature, u8 0x09, name, NUL`, padded with 0xFF),
`__types__` (**empty in the shipped files** — the hkClass reflection data is stripped,
so nothing is self-describing), `__data__` (the objects). Fixup tables: local `(from, to)`
pointer at `from` → `to` in the same section; global `(from, section, to)`; virtual
`(from, section, classNameOffset)` = the object at `from` is an instance of that class
(this is the object list). `hkArray` = `{ptr, size, capacity|flags}` (12 bytes);
`hkVector4` 16 bytes, 16-aligned; `hkTransform` = three rotation columns + translation
(64 bytes); `hkRootLevelContainer::NamedVariant` = `{name*, className*, object*}`.

The exe has no reflection either (only the hk→hkp rename table), so the class layouts
below were read off the data, validated by comparing every shape's bounds with the
render model it belongs to (the bus body collision spans −1.54..1.51 × 0.23..4.04 ×
−5.79..7.84; its model −1.54..1.51 × 0.19..4.11 × −5.86..7.85).

## Shape layouts (offsets from the object start)

| Class | Fields |
|---|---|
| `hkpShape` | 0 vtable, 4 memSize/refCount, 8 userData, 12 type |
| `hkpConvexShape` | 16 radius — the convex *margin* around the core shape |
| `hkpBoxShape` (48) | 32 halfExtents; real extent = halfExtents + radius |
| `hkpSphereShape` (32) | the radius is the sphere |
| `hkpCapsuleShape` (64) | 32 vertexA, 48 vertexB (w = radius) |
| `hkpCylinderShape` (96) | 20 cylRadius (+ convex radius), 24 base-radius factor, 32 vertexA, 48 vertexB, 64/80 perpendiculars |
| `hkpConvexVerticesShape` | 32 aabbHalfExtents, 48 aabbCenter, 64 rotatedVertices array of `hkFourTransposedPoints` (48 bytes: x of 4 points, y of 4, z of 4), 76 numVertices, 80 planeEquations array, 92 connectivity |
| `hkpConvexTranslateShape` (48) | 20 container vtable, 24 child, 32 translation |
| `hkpConvexTransformShape` (96) | 24 child, 32 hkTransform |
| `hkpTransformShape` (112) | 20 child, 32 rotation quaternion, 48 hkTransform |
| `hkpListShape` | 16 disableWelding, 24 childInfo array (16-byte entries, child at +0), 48 aabbHalfExtents, 64 aabbCenter, 80 enabledChildren[8] |
| `hkpMoppBvTreeShape` (64) | 16 MOPP code, 52 child (the real shape) |
| `hkpExtendedMeshShape` and Avalanche's `hkpStorageMorphExtendedMeshShape` (288) | 24 numBitsForSubpartIndex, 32 scaling, 48 aabbHalfExtents, 64 aabbCenter, 80 triangle-subparts array (64-byte subparts: +16 numTriangles, +24 vertexStriding, +28 numVertices, +56 index type 1/2/3 = 8/16/32-bit), 176 radius, 192 storage array (16-byte entries: storage object pointer), 204 an extra shape (a `hkpListShape` of hulls on the bus) |
| `*MeshSubpartStorage` | 8 vertices array (hkVector4), 20 indices8, 32 indices16, 44 indices32 — consecutive triples are triangles |
| `hkpRigidBody` (544) | 16 shape, 224 motion transform (rotation columns 224/240/256, translation 272), 288 swept transform (centre of mass …) |

Tree: `hkRootLevelContainer` → `hkpPhysicsData` → `hkpPhysicsSystem` → one
`hkpRigidBody` per `.pfx` → its shape. Over the test data (82 shapes of `31x14.nlz`, 12 of
the bus): 376 boxes, 437 hulls, 34 cylinders, 9 capsules, 2 spheres, 2 meshes, nothing
unsupported.

## Tile instance lists (`XXxZZ.pfi`)

```
"PFI\0", u32 1, u32 2, u32 count
f32[16]  base matrix (row-major 4×4, translation in the last row)
48 bytes of in-memory leftovers
count × { u32 hash, u32 length, char path[length], f32[16] matrix, u32 size }
```

Instance matrices are relative to the base (the first instance of `31x14.pfi` is the
village health locker at −462.4, 837.7, −8845.6, matching its placement). `size` repeats
for every instance of the same shape (its memory size?). The list is **not** the placed
objects' collision: only 7 of the 112 visible models of `31x14` have an instance at
their position. It holds the landscape props (rocks, vegetation) that are not placed as
objects; every placed object names its own shape in the `.blo` (`collision = …_col.pfx`),
and the game spawns both (277 shapes for the tile). `XXxZZ.pfs` holds only an `hkpMoppCode`, the tile's bounding
volume tree over those instances — not needed when the shapes are rebuilt.

## In jc2-game

`physics::colliders_from_body` turns a body into Avian colliders: boxes, spheres,
capsules, cylinders (stood up along their segment) and hulls merge into one compound;
parry forbids a triangle mesh inside a compound, so each mesh becomes a collider of its
own (a second static entity for tile instances, a child entity for the car's chassis —
and the wheel rays must skip those children). Tiles with a `.pfi` use it for all their
static collision; the render-mesh trimesh fallback remains for tiles without one. The car
uses the body part's `.pfx` (bus: hull + 437-triangle shell).

### Passengers (and an Avian 0.7.0 bug)

Getting out of a car crashed the release build in the solver:
`avian3d-0.7.0/src/dynamics/solver/plugin.rs:398:51`, "index out of bounds: the len
is 2 but the index is 2" (`contact_pair.manifolds[manifold_index]` in
`prepare_contact_constraints`). Rico was seated by inserting `RigidBodyDisabled` +
`ColliderDisabled` and let out by removing both and inserting a new `Position`.

**Root cause** (in Avian, two bugs that combine):

1. *Stale broad-phase flag.* The collider-tree observers that react to the removal run
   on `Discard`, i.e. *before* the components go. `add_to_tree_on::<Discard,
   ColliderDisabled>` (`collider_tree/update.rs`, case 4) re-adds the collider's proxy
   with `BODY_DISABLED = Has<RigidBodyDisabled>` — still true at that moment — and the
   case-11 observer (`On<Discard, RigidBodyDisabled>`) also reads `Has<RigidBodyDisabled>`
   (true) or skips the collider because `ColliderDisabled` is still there. So after the
   removal the proxy says `BODY_DISABLED` for good. (Removing `RigidBodyDisabled` alone
   has the same effect through case 11.)
2. *Double push.* The broad phase creates every new pair of that collider with
   `GENERATE_CONSTRAINTS` cleared (from the flag); the narrow phase sees the body is
   enabled and sets `STARTED_GENERATING_CONSTRAINTS`. If the pair is touching, the
   status loop in `narrow_phase/system_param.rs` takes the `collision_started()` branch
   first, pushes one constraint handle per manifold, and **does not clear
   `STARTED_GENERATING_CONSTRAINTS`**. The next frame the pair's manifold count changes
   (a capsule sliding along a compound car, a trimesh, a seam), its status bit is set
   again, and the `is_touching && STARTED_GENERATING_CONSTRAINTS` branch pushes a
   handle for *every* manifold once more. The edge now has more handles than the pair
   has manifolds; the solver walks the colour's handles and indexes past the end.

Sleeping has nothing to do with it (the earlier `SleepingDisabled` workaround stays —
harmless — but the test scene has it and still crashes). The tests in `physics.rs`
show both halves: `avian_reenable_leaves_proxy_flagged_disabled` (the stale flag) and
`avian_disable_reenable_still_breaks_the_solver` (the panic, 40 seeded enter/exit
cycles against a compound car); both assert the bug is *still there*, so they fail
the day Avian fixes it.

**Our fix:** `physics::stow_passenger` / `physics::release_passenger`, used by the `E`
handler (character.rs), the wreck ejection (explosion.rs) and fast travel (map.rs).
Rico's body is never disabled: stowing saves his `CollisionLayers` and `GravityScale`,
sets `CollisionLayers::NONE`, `GravityScale(0)`, zero velocity, and puts him on the
seat (`hero_physics` keeps him there). Empty layers end his contacts through the
narrow phase's ordinary "AABBs disjoint / layers don't interact" path, the one every
separating pair takes each frame, and keep him out of rays and shape casts
(`SpatialQueryFilter::test` needs a membership bit). Releasing restores both, sets
`Position` and `LinearVelocity`, and marks his proxy moved so the broad phase looks for
pairs even when he leaves where he sat. His proxy's flags are never touched, so new
pairs are born with `GENERATE_CONSTRAINTS` and never enter the buggy branch.

Rejected: re-enabling in two steps (remove `RigidBodyDisabled`, then `ColliderDisabled`
a frame later, so case 4 reads the right value) — it should work, but it hangs on the
order of Avian's observers; vendoring a patched avian3d — every agent's dependency
graph would change for a two-line upstream fix.

Upstream issue to file (avian, 0.7.0): "Removing `RigidBodyDisabled` leaves collider
proxies flagged `BODY_DISABLED`; touching pairs then get their manifolds pushed into
the constraint graph twice" — fixes: compute `is_body_disabled` in the `Discard`
observers as "being removed ⇒ false" (or move them to `Remove` and query with the
component gone), and clear `STARTED_GENERATING_CONSTRAINTS` in the `collision_started()`
branch (or test it first).

## Still unknown / later

* `hkpMoppCode` (not needed), `hkpSimpleMeshShape`/`hkpMeshShape` (not seen yet),
  `hkpTransformShape` (layout by analogy, not seen yet).
* The animation and behaviour packfiles (`animations.blz`, character rigs): different
  classes, same container.
