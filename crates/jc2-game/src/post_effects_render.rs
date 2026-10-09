//! The render-graph side of the final post pass and the auto-exposure meter (JC2-134 / JC2-137).
//!
//! This replaces Bevy's `FullscreenMaterialPlugin<PostPass>` (same fullscreen triangle, same
//! `Core3dSystems::PostProcess` slot before the tone-mapping node) because the pass needs one
//! more binding than that plugin offers: the exposure buffer.
//!
//! **Choice (documented, JC2-137):** a GPU-only chain with no CPU readback. Per view one
//! storage buffer holds the three luminance maps (64x64, 16x16, 4x4 floats) and the adapted
//! exposure (`applied`, `latest`, `inited`, see `auto_exposure.wgsl`). Each frame, in one node:
//!
//! 1. a compute pass of four dispatches (`auto_exposure.wgsl`) meters the pre-post scene
//!    (`post_process_write().source`, the 8-bit sRGB main texture) and adapts `A`;
//! 2. the fullscreen pass (`post_effects.wgsl`) applies `applied`, which is the value metered
//!    in the previous frame: **one frame of latency**, as the native pass (it runs after the
//!    composite, brief Gap 7; a possible second frame of native ring lag is not modelled).
//!
//! The meter is the pure arithmetic of `auto_exposure.rs` (tested); the WGSL mirrors it.

use crate::auto_exposure::{MAP_FINAL, MAP_INITIAL, MAP_MID};
use crate::post_effects::PostPass;
use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::FullscreenShader;
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer, BufferDescriptor,
    BufferUsages, CachedComputePipelineId, CachedRenderPipelineId, Canonical, ColorTargetState,
    ColorWrites, ComputePassDescriptor, ComputePipelineDescriptor, FilterMode, FragmentState,
    Operations, PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    Specializer, SpecializerKey, TextureFormat, TextureSampleType, Variants,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;
use core::num::NonZero;

/// Floats in the exposure buffer after the 16-byte header: the three maps.
const MAP_FLOATS: usize = MAP_INITIAL * MAP_INITIAL + MAP_MID * MAP_MID + MAP_FINAL * MAP_FINAL;
/// Size of the buffer (`struct Exposure` in `auto_exposure.wgsl`): 4 header floats + maps.
const BUFFER_BYTES: u64 = (4 + MAP_FLOATS as u64) * 4;
/// What the fragment shader binds (`struct Exposure { applied }`): the header's first float.
const FRAGMENT_BYTES: u64 = 4;

const METER_ENTRY_POINTS: [&str; 4] = ["sample_initial", "downscale_16", "downscale_4", "final_adapt"];
/// Workgroups of each dispatch (8x8 threads, the last one is a single thread).
const METER_GROUPS: [(u32, u32); 4] = [
    ((MAP_INITIAL / 8) as u32, (MAP_INITIAL / 8) as u32),
    ((MAP_MID / 8) as u32, (MAP_MID / 8) as u32),
    (1, 1),
    (1, 1),
];

pub struct PostRenderPlugin;

impl Plugin for PostRenderPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "auto_exposure.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<PostPass>::default(),
            UniformComponentPlugin::<PostPass>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ExposureBuffers>()
            .add_systems(RenderStartup, init_post_pipelines)
            .add_systems(
                Render,
                (prepare_exposure_buffers, prepare_post_pipelines).in_set(RenderSystems::Prepare),
            )
            .add_systems(
                Core3d,
                post_pass_system
                    .in_set(Core3dSystems::PostProcess)
                    .before(tonemapping),
            );
    }
}

#[derive(Resource)]
struct PostPipelines {
    /// Compute group 0: scene, linear sampler, `PostPass`, read-write exposure buffer.
    meter_layout: BindGroupLayoutDescriptor,
    /// Fragment group 0: scene, sampler, `PostPass`.
    draw_layout: BindGroupLayoutDescriptor,
    /// Fragment group 1: the exposure buffer (read only).
    exposure_layout: BindGroupLayoutDescriptor,
    /// Bilinear, clamp to edge (the native meter sampler is `MIN_MAG_LINEAR`).
    linear: Sampler,
    meter: [CachedComputePipelineId; 4],
    draw: Variants<RenderPipeline, DrawSpecializer>,
}

struct DrawSpecializer;

#[derive(PartialEq, Eq, Hash, Clone, Copy, SpecializerKey)]
struct DrawKey {
    target_format: TextureFormat,
}

impl Specializer<RenderPipeline> for DrawSpecializer {
    type Key = DrawKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.fragment_mut()?.set_target(
            0,
            ColorTargetState {
                format: key.target_format,
                blend: None,
                write_mask: ColorWrites::ALL,
            },
        );
        Ok(key)
    }
}

fn init_post_pipelines(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen_shader: Res<FullscreenShader>,
) {
    let meter_layout = BindGroupLayoutDescriptor::new(
        "post_meter_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<PostPass>(true),
                storage_buffer_sized(false, NonZero::<u64>::new(BUFFER_BYTES)),
            ),
        ),
    );
    let draw_layout = BindGroupLayoutDescriptor::new(
        "post_draw_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<PostPass>(true),
            ),
        ),
    );
    let exposure_layout = BindGroupLayoutDescriptor::new(
        "post_exposure_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (storage_buffer_read_only_sized(
                false,
                NonZero::<u64>::new(FRAGMENT_BYTES),
            ),),
        ),
    );
    let linear = render_device.create_sampler(&SamplerDescriptor {
        label: Some("post_meter_sampler"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });

    let meter_shader = asset_server.load(
        AssetPath::from_path_buf(embedded_path!("auto_exposure.wgsl")).with_source("embedded"),
    );
    let meter = METER_ENTRY_POINTS.map(|entry| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("auto_exposure_{entry}").into()),
            layout: vec![meter_layout.clone()],
            shader: meter_shader.clone(),
            entry_point: Some(entry.into()),
            ..default()
        })
    });

    let ShaderRef::Path(path) = crate::post_effects::post_shader() else {
        unreachable!("post_shader is an embedded path")
    };
    let draw_shader = asset_server.load(path);
    let draw = Variants::new(
        DrawSpecializer,
        RenderPipelineDescriptor {
            label: Some("post_effects_pipeline".into()),
            layout: vec![draw_layout.clone(), exposure_layout.clone()],
            vertex: fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: draw_shader,
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        },
    );

    commands.insert_resource(PostPipelines {
        meter_layout,
        draw_layout,
        exposure_layout,
        linear,
        meter,
        draw,
    });
}

/// Per view: the exposure buffer (zero-initialised by wgpu: `inited = 0`, so the first frame
/// takes `A = T`). Kept across frames; entries of vanished views are dropped.
#[derive(Resource, Default)]
struct ExposureBuffers(EntityHashMap<Buffer>);

fn prepare_exposure_buffers(
    device: Res<RenderDevice>,
    mut buffers: ResMut<ExposureBuffers>,
    views: Query<Entity, (With<ExtractedCamera>, With<PostPass>)>,
) {
    buffers.0.retain(|e, _| views.contains(*e));
    for entity in &views {
        buffers.0.entry(entity).or_insert_with(|| {
            device.create_buffer(&BufferDescriptor {
                label: Some("auto_exposure_state"),
                size: BUFFER_BYTES,
                usage: BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        });
    }
}

#[derive(Component)]
struct PostPipelineId(CachedRenderPipelineId);

fn prepare_post_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Option<ResMut<PostPipelines>>,
    views: Query<(Entity, &ExtractedView, Has<PostPass>), With<ExtractedCamera>>,
) -> Result<(), BevyError> {
    let Some(mut pipelines) = pipelines else {
        return Ok(());
    };
    for (entity, view, has_pass) in &views {
        if !has_pass {
            commands.entity(entity).remove::<PostPipelineId>();
            continue;
        }
        let id = pipelines.draw.specialize(
            &pipeline_cache,
            DrawKey {
                target_format: view.target_format,
            },
        )?;
        commands.entity(entity).insert(PostPipelineId(id));
    }
    Ok(())
}

fn post_pass_system(
    view: ViewQuery<(&ViewTarget, &DynamicUniformIndex<PostPass>, &PostPipelineId)>,
    pipelines: Option<Res<PostPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<PostPass>>,
    buffers: Res<ExposureBuffers>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let (view_target, uniform_index, draw_id) = view.into_inner();
    let (Some(pipelines), Some(uniform_binding), Some(buffer)) =
        (pipelines, uniforms.uniforms().binding(), buffers.0.get(&entity))
    else {
        return;
    };
    // all five pipelines are needed (the draw reads the buffer the meter fills): while one is
    // still compiling the frame is left unprocessed, like Bevy's fullscreen materials do
    let meter_ready: Option<Vec<_>> = pipelines
        .meter
        .iter()
        .map(|id| pipeline_cache.get_compute_pipeline(*id))
        .collect();
    let (Some(meter), Some(draw)) = (meter_ready, pipeline_cache.get_render_pipeline(draw_id.0))
    else {
        return;
    };

    let post_process = view_target.post_process_write();
    let device = ctx.render_device().clone();

    // 1. the meter: pre-post scene -> 64 -> 16 -> 4 -> adapted A
    let meter_group = device.create_bind_group(
        "post_meter_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.meter_layout),
        &BindGroupEntries::sequential((
            post_process.source,
            &pipelines.linear,
            uniform_binding.clone(),
            buffer.as_entire_buffer_binding(),
        )),
    );
    {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("auto_exposure"),
                timestamp_writes: None,
            });
        pass.set_bind_group(0, &meter_group, &[uniform_index.index()]);
        for (pipeline, (gx, gy)) in meter.iter().zip(METER_GROUPS) {
            pass.set_pipeline(pipeline);
            pass.dispatch_workgroups(gx, gy, 1);
        }
    }

    // 2. the composite tail with last frame's A
    let draw_group = device.create_bind_group(
        "post_draw_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.draw_layout),
        &BindGroupEntries::sequential((
            post_process.source,
            &pipelines.linear,
            uniform_binding.clone(),
        )),
    );
    let exposure_group = device.create_bind_group(
        "post_exposure_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.exposure_layout),
        &BindGroupEntries::sequential((buffer.as_entire_buffer_binding(),)),
    );
    {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("post_effects_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: post_process.destination,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations::default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(draw);
        pass.set_bind_group(0, &draw_group, &[uniform_index.index()]);
        pass.set_bind_group(1, &exposure_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buffer_layout_matches_the_wgsl() {
        // header (applied, latest, inited, pad) + 64x64 + 16x16 + 4x4
        assert_eq!(MAP_FLOATS, 4096 + 256 + 16);
        assert_eq!(BUFFER_BYTES, (4 + 4368) * 4);
        let src = include_str!("auto_exposure.wgsl");
        assert!(src.contains("lum: array<f32, 4368>"));
        assert!(src.contains("const OFF_16 = 4096u;") && src.contains("const OFF_4 = 4352u;"));
        for entry in METER_ENTRY_POINTS {
            assert!(src.contains(&format!("fn {entry}(")), "{entry}");
        }
        // 8x8 groups cover the 64x64 and 16x16 maps
        assert_eq!(METER_GROUPS[0], (8, 8));
        assert_eq!(METER_GROUPS[1], (2, 2));
    }
}
