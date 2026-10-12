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
//!
//! **JC2-138 passes before the composite** (the native "PostEffects" chain, `FUN_0096f360`):
//! `Blur` of the scene into a half-size target (spread 1.75 source texels), `Blur` again
//! (2.625 half-size texels; its result is the composite's `t1`), then `HighPass` into a
//! quarter-size target (the composite's bloom `t4`). All three targets are 8-bit sRGB like the
//! native ones. The composite also reads the camera's depth prepass (`post_effects.wgsl`).

use crate::auto_exposure::{MAP_FINAL, MAP_INITIAL, MAP_MID};
use crate::post_effects::PostPass;
use crate::water_normal::{NORMAL_MIPS, NORMAL_SIZE, WaterBump};
use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::texture::GpuImage;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponentPlugin, UniformComponentPlugin,
};
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, texture_depth_2d,
    texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
    BufferDescriptor, BufferInitDescriptor, BufferUsages, CachedComputePipelineId,
    CachedRenderPipelineId, Canonical, ColorTargetState, ColorWrites, ComputePassDescriptor,
    ComputePipelineDescriptor, Extent3d, FilterMode, FragmentState, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, Specializer, SpecializerKey,
    Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureView, TextureViewDescriptor, Variants,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderRef;
use bevy::render::render_resource::{AddressMode, LoadOp, MipmapFilterMode, StoreOp};
use core::num::NonZero;

/// Floats in the exposure buffer after the 16-byte header: the three maps.
const MAP_FLOATS: usize = MAP_INITIAL * MAP_INITIAL + MAP_MID * MAP_MID + MAP_FINAL * MAP_FINAL;
/// Size of the buffer (`struct Exposure` in `auto_exposure.wgsl`): 4 header floats + maps.
const BUFFER_BYTES: u64 = (4 + MAP_FLOATS as u64) * 4;
/// What the fragment shader binds (`struct Exposure { applied }`): the header's first float.
const FRAGMENT_BYTES: u64 = 4;

const METER_ENTRY_POINTS: [&str; 4] = [
    "sample_initial",
    "downscale_16",
    "downscale_4",
    "final_adapt",
];
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
        embedded_asset!(app, "post_blur.wgsl");
        embedded_asset!(app, "water_normal.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<PostPass>::default(),
            ExtractResourcePlugin::<WaterBump>::default(),
            UniformComponentPlugin::<PostPass>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ExposureBuffers>()
            .init_resource::<ChainTextures>()
            .add_systems(RenderStartup, (init_post_pipelines, init_water_normal))
            .add_systems(
                Render,
                (
                    prepare_exposure_buffers,
                    prepare_chain_textures,
                    prepare_post_pipelines,
                )
                    .in_set(RenderSystems::Prepare),
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
    /// Fragment group 0 of the HighPass pass: source, sampler, `PostPass`.
    draw_layout: BindGroupLayoutDescriptor,
    /// Fragment group 0 of the composite: scene, sampler, `PostPass`, blur, depth, bloom.
    /// Index 0 binds a single-sample depth texture, index 1 a multisampled one.
    composite_layout: [BindGroupLayoutDescriptor; 2],
    /// Fragment group 0 of a `Blur` pass: source, sampler, the pass step (`vec4`).
    blur_layout: BindGroupLayoutDescriptor,
    /// Fragment group 0 of the water normal passes: source, sampler, `PostPass` (phase).
    normal_layout: BindGroupLayoutDescriptor,
    /// WRAP, linear, nearest mip, unbounded LOD: the native s5 / s6 (JC2-103 section 5), also
    /// the sampler of the bump source.
    normal_sampler: Sampler,
    compose: CachedRenderPipelineId,
    downsample: CachedRenderPipelineId,
    /// Fragment group 1: the exposure buffer (read only).
    exposure_layout: BindGroupLayoutDescriptor,
    /// Bilinear, clamp to edge (the native meter sampler is `MIN_MAG_LINEAR`).
    linear: Sampler,
    meter: [CachedComputePipelineId; 4],
    blur: CachedRenderPipelineId,
    highpass: CachedRenderPipelineId,
    draw: Variants<RenderPipeline, DrawSpecializer>,
}

struct DrawSpecializer {
    composite_layout: [BindGroupLayoutDescriptor; 2],
    exposure_layout: BindGroupLayoutDescriptor,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, SpecializerKey)]
struct DrawKey {
    target_format: TextureFormat,
    /// The depth prepass of the view is multisampled (the camera uses MSAA).
    msaa_depth: bool,
}

impl Specializer<RenderPipeline> for DrawSpecializer {
    type Key = DrawKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.layout = vec![
            self.composite_layout[usize::from(key.msaa_depth)].clone(),
            self.exposure_layout.clone(),
        ];
        let fragment = descriptor.fragment_mut()?;
        if key.msaa_depth {
            fragment.shader_defs.push("MSAA_DEPTH".into());
        }
        fragment.set_target(
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

/// The generated water normal texture (`water_normal_texture`, DXGI 0x1c UNORM, 256 x 256,
/// 9 mips; native `FUN_00a8b220`) and a view per mip for the `GenerateMips` passes.
const NORMAL_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

#[derive(Resource)]
struct WaterNormal {
    /// Kept alive: the views borrow the underlying texture.
    #[allow(dead_code)]
    texture: Texture,
    full: TextureView,
    mips: Vec<TextureView>,
}

fn init_water_normal(mut commands: Commands, device: Res<RenderDevice>) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("water_normal_texture"),
        size: Extent3d {
            width: NORMAL_SIZE,
            height: NORMAL_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: NORMAL_MIPS,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: NORMAL_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let full = texture.create_view(&default());
    let mips = (0..NORMAL_MIPS)
        .map(|level| {
            texture.create_view(&TextureViewDescriptor {
                label: Some("water_normal_mip"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..default()
            })
        })
        .collect();
    commands.insert_resource(WaterNormal {
        texture,
        full,
        mips,
    });
}

/// The 8-bit sRGB format of every native blur-chain target (DXGI 0x1d, `FUN_009531d0`).
const CHAIN_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;

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
    let composite_layout = [false, true].map(|msaa| {
        let depth = if msaa {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        BindGroupLayoutDescriptor::new(
            "post_composite_layout",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, texture_2d(TextureSampleType::Float { filterable: true })),
                    (1, sampler(SamplerBindingType::Filtering)),
                    (2, uniform_buffer::<PostPass>(true)),
                    (3, texture_2d(TextureSampleType::Float { filterable: true })),
                    (4, depth),
                    (5, texture_2d(TextureSampleType::Float { filterable: true })),
                    (6, texture_2d(TextureSampleType::Float { filterable: true })),
                    (7, sampler(SamplerBindingType::Filtering)),
                ),
            ),
        )
    });
    let blur_layout = BindGroupLayoutDescriptor::new(
        "post_blur_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<Vec4>(false),
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
    let normal_sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("water_normal_sampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
        ..default()
    });
    let normal_layout = draw_layout.clone();

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
    let chain_target = || {
        vec![Some(ColorTargetState {
            format: CHAIN_FORMAT,
            blend: None,
            write_mask: ColorWrites::ALL,
        })]
    };
    let blur_shader = asset_server
        .load(AssetPath::from_path_buf(embedded_path!("post_blur.wgsl")).with_source("embedded"));
    let blur = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("post_blur_pipeline".into()),
        layout: vec![blur_layout.clone()],
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: blur_shader,
            targets: chain_target(),
            ..default()
        }),
        ..default()
    });
    let highpass = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("post_highpass_pipeline".into()),
        layout: vec![draw_layout.clone(), exposure_layout.clone()],
        vertex: fullscreen_shader.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: draw_shader.clone(),
            entry_point: Some("highpass".into()),
            targets: chain_target(),
            ..default()
        }),
        ..default()
    });
    let normal_shader = asset_server
        .load(AssetPath::from_path_buf(embedded_path!("water_normal.wgsl")).with_source("embedded"));
    let normal_pipeline = |entry: &'static str| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("water_normal_{entry}").into()),
            layout: vec![normal_layout.clone()],
            vertex: fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: normal_shader.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format: NORMAL_FORMAT,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        })
    };
    let compose = normal_pipeline("compose");
    let downsample = normal_pipeline("downsample");
    let draw = Variants::new(
        DrawSpecializer {
            composite_layout: composite_layout.clone(),
            exposure_layout: exposure_layout.clone(),
        },
        RenderPipelineDescriptor {
            label: Some("post_effects_pipeline".into()),
            layout: vec![composite_layout[0].clone(), exposure_layout.clone()],
            vertex: fullscreen_shader.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: draw_shader,
                entry_point: Some("fragment".into()),
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
        composite_layout,
        blur_layout,
        normal_layout,
        normal_sampler,
        compose,
        downsample,
        exposure_layout,
        linear,
        meter,
        blur,
        highpass,
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

/// The blur-chain targets of one view (native `FUN_009531d0`: two half-size and one
/// quarter-size 8-bit sRGB textures) and the per-pass `Blur` steps.
struct ChainTargets {
    /// The size of the view target these were made for.
    size: UVec2,
    /// `large_temp_texture[0]` / `[1]`: blur pass 0 writes the first, pass 1 the second.
    half: [(Texture, TextureView); 2],
    /// `small_temp_texture[0]`: the HighPass output (the composite's bloom `t4`).
    quarter: (Texture, TextureView),
    /// `Blur` uniform of pass 0 and pass 1 (`blur_step`).
    steps: [Buffer; 2],
}

#[derive(Resource, Default)]
struct ChainTextures(EntityHashMap<ChainTargets>);

/// Half and quarter extents of the native chain targets (integer division, at least 1).
fn chain_sizes(size: UVec2) -> (UVec2, UVec2) {
    ((size / 2).max(UVec2::ONE), (size / 4).max(UVec2::ONE))
}

/// The `Blur` step `(spread / width, spread / height)` of pass `pass` for a view of `size`
/// (`FUN_0092ec50`: pass 0 uses the full-size dimensions and 1.75 texels, pass 1 the half-size
/// target and 2.625 texels).
pub fn blur_step(size: UVec2, pass: usize) -> Vec2 {
    let (half, _) = chain_sizes(size);
    let (spread, dims) = match pass {
        0 => (1.75, size),
        _ => (2.625, half),
    };
    Vec2::new(spread / dims.x as f32, spread / dims.y as f32)
}

fn prepare_chain_textures(
    device: Res<RenderDevice>,
    mut chains: ResMut<ChainTextures>,
    views: Query<(Entity, &ExtractedCamera), With<PostPass>>,
) {
    chains.0.retain(|e, _| views.contains(*e));
    for (entity, camera) in &views {
        let Some(size) = camera.physical_target_size else {
            continue;
        };
        if chains.0.get(&entity).is_some_and(|c| c.size == size) {
            continue;
        }
        let make = |label: &'static str, dims: UVec2| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: dims.x,
                    height: dims.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: CHAIN_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&default());
            (texture, view)
        };
        let (half, quarter) = chain_sizes(size);
        let step = |pass: usize| {
            let v = blur_step(size, pass);
            let mut bytes = Vec::with_capacity(16);
            for f in [v.x, v.y, 0.0, 0.0] {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("post_blur_step"),
                contents: &bytes,
                usage: BufferUsages::UNIFORM,
            })
        };
        chains.0.insert(
            entity,
            ChainTargets {
                size,
                half: [
                    make("post_large_temp_texture_0", half),
                    make("post_large_temp_texture_1", half),
                ],
                quarter: make("post_small_temp_texture", quarter),
                steps: [step(0), step(1)],
            },
        );
    }
}

#[derive(Component)]
struct PostPipelineId(CachedRenderPipelineId);

/// What `prepare_post_pipelines` reads of a view.
type PostViewItem = (
    Entity,
    &'static ExtractedView,
    Has<PostPass>,
    Option<&'static ViewPrepassTextures>,
);

fn prepare_post_pipelines(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Option<ResMut<PostPipelines>>,
    views: Query<PostViewItem, With<ExtractedCamera>>,
) -> Result<(), BevyError> {
    let Some(mut pipelines) = pipelines else {
        return Ok(());
    };
    for (entity, view, has_pass, prepass) in &views {
        // the composite reads the depth prepass: a view without one is left unprocessed
        let depth_samples = prepass
            .and_then(|p| p.depth.as_ref())
            .map(|d| d.texture.texture.sample_count());
        let (true, Some(samples)) = (has_pass, depth_samples) else {
            commands.entity(entity).remove::<PostPipelineId>();
            continue;
        };
        let id = pipelines.draw.specialize(
            &pipeline_cache,
            DrawKey {
                target_format: view.target_format,
                msaa_depth: samples > 1,
            },
        )?;
        commands.entity(entity).insert(PostPipelineId(id));
    }
    Ok(())
}

/// The water normal texture inputs of the post pass.
#[derive(bevy::ecs::system::SystemParam)]
struct WaterParams<'w> {
    normal: Option<Res<'w, WaterNormal>>,
    bump: Option<Res<'w, WaterBump>>,
    gpu_images: Res<'w, RenderAssets<GpuImage>>,
}

fn post_pass_system(
    view: ViewQuery<(
        &ViewTarget,
        &DynamicUniformIndex<PostPass>,
        &PostPipelineId,
        &ViewPrepassTextures,
    )>,
    pipelines: Option<Res<PostPipelines>>,
    pipeline_cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<PostPass>>,
    (buffers, chains): (Res<ExposureBuffers>, Res<ChainTextures>),
    water: WaterParams,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let (view_target, uniform_index, draw_id, prepass) = view.into_inner();
    let WaterParams {
        normal,
        bump,
        gpu_images,
    } = water;
    let (
        Some(pipelines),
        Some(uniform_binding),
        Some(buffer),
        Some(chain),
        Some(depth_view),
        Some(normal),
    ) = (
        pipelines,
        uniforms.uniforms().binding(),
        buffers.0.get(&entity),
        chains.0.get(&entity),
        prepass.depth_view(),
        normal,
    ) else {
        return;
    };
    // all pipelines are needed (the draw reads the buffer the meter fills): while one is
    // still compiling the frame is left unprocessed, like Bevy's fullscreen materials do
    let meter_ready: Option<Vec<_>> = pipelines
        .meter
        .iter()
        .map(|id| pipeline_cache.get_compute_pipeline(*id))
        .collect();
    let (Some(meter), Some(draw), Some(blur), Some(highpass), Some(compose), Some(downsample)) = (
        meter_ready,
        pipeline_cache.get_render_pipeline(draw_id.0),
        pipeline_cache.get_render_pipeline(pipelines.blur),
        pipeline_cache.get_render_pipeline(pipelines.highpass),
        pipeline_cache.get_render_pipeline(pipelines.compose),
        pipeline_cache.get_render_pipeline(pipelines.downsample),
    ) else {
        return;
    };
    let msaa_depth = prepass
        .depth
        .as_ref()
        .is_some_and(|d| d.texture.texture.sample_count() > 1);

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

    // 2. the blur chain: scene -> half (1.75 texel spread) -> half (2.625 texels)
    let blur_layout = pipeline_cache.get_bind_group_layout(&pipelines.blur_layout);
    let chain_pass = |ctx: &mut RenderContext,
                      label: &'static str,
                      pipeline: &RenderPipeline,
                      groups: &[(&BindGroup, &[u32])],
                      target: &TextureView| {
        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations::default(),
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        pass.set_pipeline(pipeline);
        for (i, (group, offsets)) in groups.iter().enumerate() {
            pass.set_bind_group(i as u32, *group, offsets);
        }
        pass.draw(0..3, 0..1);
    };
    // 1b. the water normal texture (`water_normal_texture`): four scrolling layers of the bump
    // image into mip 0, then the mip chain. Without the bump image (no game data) it holds the
    // neutral normal (0.5, 0.5, 0, 0): no distortion, no caustics.
    let normal_layout = pipeline_cache.get_bind_group_layout(&pipelines.normal_layout);
    let bump_view = bump
        .as_ref()
        .and_then(|b| gpu_images.get(&b.0))
        .map(|g| &g.texture_view);
    if let Some(bump_view) = bump_view {
        let compose_group = device.create_bind_group(
            "water_normal_compose_bind_group",
            &normal_layout,
            &BindGroupEntries::sequential((
                bump_view,
                &pipelines.normal_sampler,
                uniform_binding.clone(),
            )),
        );
        chain_pass(
            &mut ctx,
            "water_normal_compose",
            compose,
            &[(&compose_group, &[uniform_index.index()])],
            &normal.mips[0],
        );
        for level in 1..normal.mips.len() {
            let group = device.create_bind_group(
                "water_normal_mip_bind_group",
                &normal_layout,
                &BindGroupEntries::sequential((
                    &normal.mips[level - 1],
                    &pipelines.linear,
                    uniform_binding.clone(),
                )),
            );
            chain_pass(
                &mut ctx,
                "water_normal_mip",
                downsample,
                &[(&group, &[uniform_index.index()])],
                &normal.mips[level],
            );
        }
    } else {
        for view in &normal.mips {
            let _clear = ctx
                .command_encoder()
                .begin_render_pass(&RenderPassDescriptor {
                    label: Some("water_normal_neutral"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(LinearRgba::new(0.5, 0.5, 0.0, 0.0).into()),
                            store: StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
        }
    }

    let blur0_group = device.create_bind_group(
        "post_blur0_bind_group",
        &blur_layout,
        &BindGroupEntries::sequential((
            post_process.source,
            &pipelines.linear,
            chain.steps[0].as_entire_buffer_binding(),
        )),
    );
    chain_pass(
        &mut ctx,
        "post_blur_0",
        blur,
        &[(&blur0_group, &[])],
        &chain.half[0].1,
    );
    let blur1_group = device.create_bind_group(
        "post_blur1_bind_group",
        &blur_layout,
        &BindGroupEntries::sequential((
            &chain.half[0].1,
            &pipelines.linear,
            chain.steps[1].as_entire_buffer_binding(),
        )),
    );
    chain_pass(
        &mut ctx,
        "post_blur_1",
        blur,
        &[(&blur1_group, &[])],
        &chain.half[1].1,
    );

    // 3. HighPass: blurred half-size scene -> quarter-size bloom texture (reads A)
    let exposure_group = device.create_bind_group(
        "post_exposure_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.exposure_layout),
        &BindGroupEntries::sequential((buffer.as_entire_buffer_binding(),)),
    );
    let highpass_group = device.create_bind_group(
        "post_highpass_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.draw_layout),
        &BindGroupEntries::sequential((
            &chain.half[1].1,
            &pipelines.linear,
            uniform_binding.clone(),
        )),
    );
    chain_pass(
        &mut ctx,
        "post_highpass",
        highpass,
        &[
            (&highpass_group, &[uniform_index.index()]),
            (&exposure_group, &[]),
        ],
        &chain.quarter.1,
    );

    // 4. the composite with last frame's A
    let draw_group = device.create_bind_group(
        "post_draw_bind_group",
        &pipeline_cache.get_bind_group_layout(&pipelines.composite_layout[usize::from(msaa_depth)]),
        &BindGroupEntries::with_indices((
            (0, post_process.source),
            (1, &pipelines.linear),
            (2, uniform_binding.clone()),
            (3, &chain.half[1].1),
            (4, depth_view),
            (5, &chain.quarter.1),
            (6, &normal.full),
            (7, &pipelines.normal_sampler),
        )),
    );
    chain_pass(
        &mut ctx,
        "post_effects_pass",
        draw,
        &[
            (&draw_group, &[uniform_index.index()]),
            (&exposure_group, &[]),
        ],
        post_process.destination,
    );
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

    /// The `Blur` steps of `FUN_0092ec50`: 1.75 full-size texels, then 2.625 half-size texels.
    #[test]
    fn the_blur_steps_follow_the_native_spreads() {
        let size = UVec2::new(1920, 1080);
        let s0 = blur_step(size, 0);
        assert!((s0.x - 1.75 / 1920.0).abs() < 1e-9 && (s0.y - 1.75 / 1080.0).abs() < 1e-9);
        let s1 = blur_step(size, 1);
        assert!((s1.x - 2.625 / 960.0).abs() < 1e-9 && (s1.y - 2.625 / 540.0).abs() < 1e-9);
        // odd sizes: integer division, as the native `param / 2` and `/ 4`
        let (half, quarter) = chain_sizes(UVec2::new(1281, 721));
        assert_eq!(
            (half, quarter),
            (UVec2::new(640, 360), UVec2::new(320, 180))
        );
        assert_eq!(chain_sizes(UVec2::ONE), (UVec2::ONE, UVec2::ONE));
    }

    /// The WGSL tap table is the one the exe stores (float32 bit patterns read from the
    /// constants at 0xe5becc..0xe5d5e8 by `FUN_0092ec50`, JC2-138 probe notes).
    #[test]
    fn the_blur_tap_table_matches_the_exe_constants() {
        let src = include_str!("post_blur.wgsl");
        let bits: [(u32, u32); 6] = [
            (0x3d77_5ea6, 0x3e1f_1173),
            (0xbe99_0be9, 0xbe17_0c13),
            (0x3ef5_cdb3, 0xbe0f_0f9d),
            (0xbeb1_4035, 0x3f11_d9f5),
            (0xbe25_d3dd, 0xbf51_4435),
            (0x3f4e_4e6a, 0x3f17_9229),
        ];
        assert!(src.contains("vec2<f32>(0.0, 0.0),"));
        for (x, y) in bits {
            let line = format!(
                "vec2<f32>({:?}, {:?}),",
                f32::from_bits(x),
                f32::from_bits(y)
            );
            assert!(src.contains(&line), "missing tap {line}");
        }
        assert!(src.contains("0.142857"));
    }
}
