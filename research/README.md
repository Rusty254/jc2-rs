# research/ — reverse-engineering notes on the original JustCause2.exe

This folder is the reference material for the Rust reimplementation in the parent
directory. Nothing here is compiled or shipped; it exists so that when a format or a
behaviour needs to be understood, the answer can be read off the original engine
instead of guessed.

Research for roadmap items 7–9 (mission access, larger missions, world and vehicles)
is indexed in [mission-world-roadmap.md](mission-world-roadmap.md), with separate
evidence briefs and an implementation sequence. These are research findings;
the proposed runtime changes have not been implemented or tested.

The seven additional assignments in `notes/RESEARCH-HANDOFFS.md` are indexed in
[additional-research-roadmap.md](additional-research-roadmap.md), including
AI dialogue, material parameters, water, particles, IK, weapon sockets and saves.

Keep game code out of the repo: the `.gzf` and `pseudo/` output are derived from your
own copy of the exe and are git-ignored.

## What is here

```
research/
  README.md
  analysis/
    pe_fingerprint.txt/json   toolchain, imports, sections, PDB path, RTTI summary
    rtti_classes.txt          3,708 class names (Avalanche N* namespaces, Havok hk*, Anark)
    imports.txt               every imported DLL function
    functions.csv             41,376 functions: address, size, name, namespace, signature, cconv
    classes.csv               3,782 classes with vftable addresses and member counts
    strings.csv               19,809 strings with xref counts — the fastest way into any subsystem
    calls.csv                 119,885 static call edges (caller -> callee)
    pseudo/                   Ghidra decompiler output for every function
      _unnamed/text_XXXXXXXX.c   functions with no symbol, grouped per 64 KB of .text
      _global/named_*.c          functions Ghidra could name (CRT, imports, FID matches)
      <Namespace>/<Class>.cpp    the few with recovered namespaces
  ghidra/
    JustCause2.exe.gzf        the full analysed program — File > Import in Ghidra 12.x
  tools/
    ghidra_12.1.4_PUBLIC_20260921.zip   Ghidra itself (needs a JDK 21, e.g. Adoptium Temurin)
    ghidra_scripts/ExportDecompProject.java   regenerates analysis/ from the .gzf (headless)
    pe_fingerprint.py                         regenerates pe_fingerprint.*
```

## The binary in one paragraph

`JustCause2.exe` (14,549,656 bytes, sha256 `df99a479…b74f80`) was built 2010-06-18
with Visual C++ 2005 SP1 (cl 14.00.50727, link 8.0) from 1026 C++ and 65 C objects,
without LTCG or PGO, against the dynamic CRT (`MSVCR80`/`MSVCP80`). Image base
`0x400000`, no ASLR, no relocations, so every address in these files is the runtime
address. Entry point `0x405290` is the CRT's `WinMainCRTStartup`; the game's `WinMain`
is `0x0042d5e0` (5,571 bytes). Statically linked middleware: Havok (physics,
animation, behaviour — about 450 `hk*` classes), Anark Gameface (UI), Boost 1.39
headers. Dynamic: FMOD Ex + FMOD Event, PathEngine, Bink, CUDA (water simulation),
Steamworks, DirectX 9/10 (`d3d9.dll` import, DX10 shader bundle on disk).

Avalanche's own code lives in `N<System>` namespaces — `NAnimationSystem` (211
classes), `NGraphicsEngine` (157), `NCtgAI` (122), `NSoundLibrary` (90), `NCutscene`
(71), `NPaperDoll`, `NCharacterSystem`, `NModelSystem`, `NMissionSystem`, `NNetwork`,
`NAccomplishment`, `NRuleSystem`, `NParticleSystem`, `NSpawning` — with classes
prefixed `C` (1,127 global `C*` classes) and interfaces prefixed `I`. The `VC*`
classes (`VCSpawnManager`, `VCHeatControl`, `VCCivilianRoamArea`, …) are the
scriptable world objects that the location property files instantiate.

## How to use it when writing the Rust engine

1. Start from data. `jc2tool prop .. some_file.bin` shows the keys the engine reads.
   Hash a guessed key name with `jc2tool hash <name>`; if it matches a `#xxxxxxxx`
   key, add it to `crates/jc2-formats/data/property_names.txt`.
2. Find the consumer. `grep -i "camera_settings" analysis/strings.csv` gives the string
   address; `grep -l <address> analysis/pseudo/_unnamed/*.c` finds the function that
   references it (or open the `.gzf` in Ghidra and use References > Show References).
3. Read the pseudo-C for the loading routine — the order it reads fields and the types
   it converts them to is the file format specification.
4. For structures, retype them in Ghidra as you go (Data Type Manager); re-run the
   export script afterwards to refresh `pseudo/`.
5. Constants matter as much as code: read `DAT_xxxxxxxx` globals straight out of the
   exe's `.rdata` (a few lines of Python with the section table) rather than guessing —
   the terrain height scale is `2200/65535`, not the `1/32` a round number would
   suggest, and that one guess hid the terrain orientation for a while.

Worked examples are in `formats/`: `rbm.md` (models), `terrain.md` (height fields,
world layout) and `world.md` (location containers, placements).

Re-exporting after Ghidra work:

```
<ghidra>\support\analyzeHeadless <project_dir> JC2 -process JustCause2.exe -noanalysis ^
   -scriptPath research\tools\ghidra_scripts -postScript ExportDecompProject.java research\analysis
```

## Sources

Toolchain identification from the PE Rich header
([decoding MSVC build numbers](https://devblogs.microsoft.com/cppblog/what-compiler-does-the-compiler-team-use-and-decoding-version-numbers/)).
Archive/SARC/property formats cross-checked against
[Gibbed.Avalanche](https://github.com/gibbed/Gibbed.Avalanche) (zlib licence).
