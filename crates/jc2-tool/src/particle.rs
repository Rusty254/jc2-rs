use anyhow::Result;
use jc2_formats::particle::{FIXTURE_EFFECT, FIXTURE_LEAF, ParticleFixture};
use std::path::Path;
pub fn inspect(game_dir: &Path, json: bool) -> Result<()> {
    let fs = jc2_formats::GameFs::open(game_dir)?;
    let f = ParticleFixture::load(&fs, FIXTURE_EFFECT, FIXTURE_LEAF)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "effect_file": FIXTURE_EFFECT, "effect": f.effect,
                "leaf": f.system, "texture_member": f.texture_member,
                "remaining_approximations": ["graph routing", "child link", "sound", "native birth RNG/direction", "fitted cell geometry", "native material mode", "native job clock/publication"]
            }))?
        );
    } else {
        let d = jc2_formats::dds::DdsImage::parse(&f.texture_bytes)?;
        println!(
            "{FIXTURE_EFFECT} -> {FIXTURE_LEAF}.psmb -> {} ({}x{}, {:?})",
            f.texture_member, d.width, d.height, d.format
        );
        println!(
            "rate={} lifetime={} system_lifetime={} atlas={:?} space={} initial_alpha={} size={:?} frame={:?}",
            f.system.emission_rate,
            f.system.lifetime,
            f.system.system_lifetime,
            f.system.atlas,
            f.system.space,
            f.system.initial_alpha,
            f.system.initial_size,
            f.system.initial_frame
        );
        for c in &f.effect.components {
            println!(
                "component path={:x?} type={} id={:?} particle_names={:?}",
                c.path, c.kind, c.id, c.particle_names
            );
        }
        for (index, m) in f.system.execution_modifiers().enumerate() {
            println!(
                "indexed modifier {} #{:08x} {} normalized window={}..{} typed={:?} raw values={:?}",
                index, m.key, m.class, m.start, m.end, m.kind, m.raw.values
            );
        }
        println!(
            "raw child link {:?}; graph/sound/birth direction/RNG/fitted geometry/material/job-clock parity unresolved; named viewer evaluates indexed updates and local space2",
            f.system.child_link
        );
    }
    Ok(())
}
