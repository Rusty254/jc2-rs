//! Bevy asset loader for `.rbm` render-block models.
//!
//! `asset_server.load("jc2://lave.v046_buscoach.ee/v046-body_m_lod1.rbm")`
//! yields an [`RbmAsset`] whose blocks are already Bevy meshes and
//! `StandardMaterial`s, with textures loaded from the same container (or the
//! archive top level, see `archive_source`). RGB normal maps are enabled only
//! for verified block families and are separate, linear image subassets.

use crate::coords::{to_bevy, winding_to_bevy};
use bevy::asset::io::{AssetSourceId, Reader};
use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages};
use bevy::image::ImageLoaderSettings;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use jc2_formats::rbm::{RbmModel, RenderBlock};
use jc2_formats::texture_policy::{GAMMA_CORRECT_FILE, GammaExceptions, GammaMode, dds_view_pair};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Provisional silhouette repair. The original shader also scales alpha by
/// instance fade; its 0.5 reference is not a plain texture-alpha cutoff.
const FOLIAGE_ALPHA_CUTOFF: f32 = 0.5;

fn alpha_mode(type_name: &str) -> AlphaMode {
    match type_name {
        "VegetationFoliage" | "BillboardFoliage" => AlphaMode::Mask(FOLIAGE_ALPHA_CUTOFF),
        "DeformableWindow" | "Window" => AlphaMode::Blend,
        _ => AlphaMode::Opaque,
    }
}

/// These shaders sample normal RGB as 2*RGB-1. Other block families, packed
/// properties/AOC maps and the foliage diffuse-as-normal placeholder are not
/// interpreted here. See material-parameters-next.md for the shader evidence.
fn normal_texture_name(block: &RenderBlock, has_tangents: bool) -> Option<&str> {
    let normal = &block.material.normal;
    (has_tangents
        && matches!(
            block.type_name.as_str(),
            "General" | "CarPaint" | "VegetationBark"
        )
        && !normal.is_empty()
        && !normal.eq_ignore_ascii_case(&block.material.diffuse))
    .then_some(normal)
}

/// Preserve the authored direction, removing packed quantization's component
/// along N. An invalid/degenerate basis disables the whole tangent stream rather
/// than inventing a sign or feeding NaNs to the PBR shader.
fn mesh_tangents(block: &RenderBlock, normals: &[[f32; 3]]) -> Option<Vec<[f32; 4]>> {
    let md = &block.mesh;
    if md.tangents.len() != md.positions.len() || normals.len() != md.positions.len() {
        return None;
    }
    md.tangents
        .iter()
        .zip(normals)
        .map(|(t, n)| {
            let n = Vec3::from(*n).try_normalize()?;
            let tangent = to_bevy([t[0], t[1], t[2]]);
            if !tangent.is_finite() || !t[3].is_finite() || t[3].abs() < 1e-6 {
                return None;
            }
            let tangent = (tangent - n * tangent.dot(n)).try_normalize()?;
            // General VS 188280 / PS 6062008 and Bark_Water reconstruct
            // B = sign(packed_tangent) * cross(N,T), matching Bevy. DXBC `lt`
            // returns an all-bit mask (-1), not integer 1. Coordinates are
            // identity, so both authored signs survive without another flip.
            let sign = t[3].signum();
            Some(tangent.extend(sign).to_array())
        })
        .collect()
}

#[derive(Asset, TypePath, Debug)]
pub struct RbmAsset {
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
    pub blocks: Vec<RbmBlock>,
    pub truncated_at: Option<String>,
}

#[derive(Debug)]
pub struct RbmBlock {
    pub type_name: String,
    pub diffuse: String,
    /// Colour image policy and resource/view diagnostics for this exact load.
    pub diffuse_view: Option<RbmTextureView>,
    pub foliage: Option<RbmFoliage>,
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    pub triangles: usize,
    /// The mesh carries joint indices and weights (`SkinnedGeneral`).
    pub skinned: bool,
}

#[derive(Clone, Debug)]
pub struct RbmFoliage {
    pub material: Handle<crate::foliage_alpha::FoliageMaterial>,
    pub state: jc2_formats::foliage::FoliageAlpha,
}
impl RbmBlock {
    pub fn material_component(&self) -> crate::foliage_alpha::RbmMaterial {
        self.material_component_with(self.material.clone())
    }
    pub fn material_component_with(
        &self,
        standard: Handle<StandardMaterial>,
    ) -> crate::foliage_alpha::RbmMaterial {
        crate::foliage_alpha::RbmMaterial {
            standard,
            foliage: self.foliage.as_ref().map(|f| (f.material.clone(), f.state)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RbmTextureView {
    pub raw_name: String,
    pub asset_path: String,
    pub source_dxgi: Option<u32>,
    pub selected_dxgi: Option<u32>,
    pub is_srgb: bool,
    pub fallback: Option<String>,
}

#[derive(Hash, PartialEq, Eq)]
struct ColourImageKey {
    raw_name: String,
    asset_path: String,
    family: String,
}

#[derive(Debug)]
struct SourcePolicy {
    exceptions: GammaExceptions,
    limitation: Option<String>,
}

#[derive(TypePath)]
pub struct RbmLoader {
    mode: GammaMode,
    // Per-loader, per-source: never share a process-global exception list across
    // independently mounted GameFs instances/AssetServers. Mounts are immutable;
    // refreshing the exception asset requires a fresh loader/application.
    policies: Mutex<HashMap<AssetSourceId<'static>, Arc<SourcePolicy>>>,
}

impl Default for RbmLoader {
    fn default() -> Self {
        Self::new(GammaMode::AuthoredExceptions)
    }
}

impl RbmLoader {
    pub fn new(mode: GammaMode) -> Self {
        Self {
            mode,
            policies: Mutex::new(HashMap::new()),
        }
    }

    async fn source_policy(&self, cx: &mut LoadContext<'_>) -> Arc<SourcePolicy> {
        let source = cx.path().source().clone_owned();
        let cached = self.policies.lock().unwrap().get(&source).cloned();
        if let Some(policy) = cached {
            return policy;
        }
        let policy = if self.mode == GammaMode::OnlyTextureNames {
            SourcePolicy {
                exceptions: GammaExceptions::default(),
                limitation: None,
            }
        } else {
            let path = bevy::asset::AssetPath::from(GAMMA_CORRECT_FILE).with_source(source.clone());
            match cx.read_asset_bytes(path).await {
                Ok(bytes) => match GammaExceptions::parse(&bytes) {
                    Ok(exceptions) => SourcePolicy {
                        exceptions,
                        limitation: None,
                    },
                    Err(error) => SourcePolicy {
                        exceptions: GammaExceptions::default(),
                        limitation: Some(format!(
                            "malformed gamma exceptions: {error}; name-only fallback"
                        )),
                    },
                },
                Err(error) => SourcePolicy {
                    exceptions: GammaExceptions::default(),
                    limitation: Some(format!(
                        "gamma exceptions unavailable: {error}; name-only fallback"
                    )),
                },
            }
        };
        if let Some(limitation) = &policy.limitation {
            warn!("RBM texture policy source={source}: {limitation}");
        } else {
            debug!(
                "RBM texture policy source={source} mode={:?} exceptions={}",
                self.mode,
                policy.exceptions.len()
            );
        }
        self.policies
            .lock()
            .unwrap()
            .entry(source)
            .or_insert_with(|| Arc::new(policy))
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RbmLoadError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("rbm: {0}")]
    Format(#[from] jc2_formats::Error),
    #[error("required diffuse {raw_name} at {asset_path}: {message}")]
    Diffuse {
        raw_name: String,
        asset_path: String,
        message: String,
    },
}

pub fn block_to_mesh(block: &RenderBlock) -> Mesh {
    let md = &block.mesh;
    let positions: Vec<[f32; 3]> = md.positions.iter().map(|p| to_bevy(*p).into()).collect();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let n = positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    let normals: Vec<[f32; 3]> = if md.normals.len() == n {
        md.normals.iter().map(|v| to_bevy(*v).into()).collect()
    } else {
        // from the triangles, in Bevy space (the same winding: `coords::winding_to_bevy`)
        let mut tmp = md.clone();
        for p in &mut tmp.positions {
            *p = to_bevy(*p).into();
        }
        tmp.compute_normals();
        tmp.normals
    };
    if let Some(tangents) = mesh_tangents(block, &normals) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    if md.uv0.len() == n {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, md.uv0.clone());
    } else {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
    }
    if md.uv1.len() == n && md.uv1.iter().flatten().all(|v| v.is_finite()) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, md.uv1.clone());
    }
    if md.joints.len() == n && md.weights.len() == n {
        // skinned: a vertex nobody weights hangs off its first joint
        let weights: Vec<[f32; 4]> = md
            .weights
            .iter()
            .map(|w| {
                let sum: f32 = w.iter().sum();
                if sum > 1e-4 {
                    [w[0] / sum, w[1] / sum, w[2] / sum, w[3] / sum]
                } else {
                    [1.0, 0.0, 0.0, 0.0]
                }
            })
            .collect();
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(md.joints.clone()),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    }
    let mut indices = md.indices.clone();
    winding_to_bevy(&mut indices);
    mesh.insert_indices(Indices::U16(indices));
    mesh
}

impl AssetLoader for RbmLoader {
    type Asset = RbmAsset;
    type Settings = ();
    type Error = RbmLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        cx: &mut LoadContext<'_>,
    ) -> Result<RbmAsset, RbmLoadError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let model = RbmModel::parse(&bytes)?;

        // "container.ee/model.rbm" -> textures are "container.ee/<name>"; a bare
        // "model.rbm" at the top level resolves textures at the top level too.
        let path = cx.path().clone();
        let source = path.source().clone();
        let parent = path
            .path()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();

        let policy = self.source_policy(cx).await;

        let mut blocks = Vec::with_capacity(model.blocks.len());
        let mut normal_images: HashMap<String, Option<Handle<Image>>> = HashMap::new();
        let mut colour_images: HashMap<ColourImageKey, (Option<Handle<Image>>, RbmTextureView)> =
            HashMap::new();
        for (i, block) in model.blocks.iter().enumerate() {
            if block.mesh.positions.is_empty() {
                continue;
            }
            let mesh_value = block_to_mesh(block);
            let has_tangents = mesh_value.contains_attribute(Mesh::ATTRIBUTE_TANGENT)
                && block.mesh.uv0.len() == block.mesh.positions.len()
                && block.mesh.uv0.iter().flatten().all(|v| v.is_finite());
            let mesh = cx.add_labeled_asset(format!("mesh{i}"), mesh_value);
            let diffuse = block.material.diffuse.clone();
            let (texture, diffuse_view) = if diffuse.is_empty() {
                (None, None)
            } else {
                // Keep the callback input unchanged. Path separator normalization
                // is for archive lookup only, after name-policy selection.
                let tex_path =
                    bevy::asset::AssetPath::from_path_buf(parent.join(diffuse.replace('\\', "/")))
                        .with_source(source.clone());
                // Family participates in eligibility and fallback diagnostics, even
                // when two blocks request the exact same raw name and path.
                let key = ColourImageKey {
                    raw_name: diffuse.clone(),
                    asset_path: tex_path.to_string(),
                    family: block.type_name.clone(),
                };
                if let Some((handle, view)) = colour_images.get(&key) {
                    (handle.clone(), Some(view.clone()))
                } else {
                    let mut view = RbmTextureView {
                        raw_name: diffuse.clone(),
                        asset_path: tex_path.to_string(),
                        source_dxgi: None,
                        selected_dxgi: None,
                        is_srgb: true,
                        fallback: policy.limitation.clone(),
                    };
                    let selection = if matches!(
                        block.type_name.as_str(),
                        "General" | "CarPaint" | "VegetationBark" | "VegetationFoliage"
                    ) {
                        match policy.exceptions.select(&diffuse) {
                            Ok(name) => match cx.read_asset_bytes(tex_path.clone()).await {
                                Ok(bytes) => dds_view_pair(&bytes, name.is_srgb)
                                    .map(|pair| (name, pair))
                                    .map_err(|e| e.to_string()),
                                Err(error) => Err(format!("DDS metadata unavailable: {error}")),
                            },
                            Err(error) => Err(error.to_string()),
                        }
                    } else {
                        Err(format!("unverified block family {}", block.type_name))
                    };
                    match selection {
                        Ok((name, pair)) => {
                            view.is_srgb = name.is_srgb;
                            view.source_dxgi = Some(pair.source_dxgi);
                            view.selected_dxgi = Some(pair.selected_dxgi);
                            debug!(
                                "RBM {path}: colour raw={} asset={} rule={:?} DDS={} resource/SRV={} srgb={} limitation={:?}",
                                view.raw_name,
                                view.asset_path,
                                name.rule,
                                pair.source_dxgi,
                                pair.selected_dxgi,
                                view.is_srgb,
                                view.fallback
                            );
                        }
                        Err(error) => {
                            view.fallback = Some(format!("{error}; legacy sRGB image fallback"));
                            warn!(
                                "RBM {path}: colour raw={} asset={}: {}",
                                view.raw_name,
                                view.asset_path,
                                view.fallback.as_ref().unwrap()
                            );
                        }
                    }
                    let is_srgb = view.is_srgb;
                    // Settings are not part of a deferred image path's identity.
                    // A directly loaded value and RBM-local label isolate this
                    // colour view from normal/data/default-path image handles.
                    let image = cx
                        .load_builder()
                        .with_settings(move |settings: &mut ImageLoaderSettings| {
                            settings.is_srgb = is_srgb
                        })
                        .load_value::<Image>(tex_path)
                        .await;
                    let handle = match image {
                        Ok(image) => {
                            let image = image.take();
                            if image.texture_descriptor.format.is_srgb() != view.is_srgb {
                                view.fallback = Some(
                                    "image decoder view differs from requested colour policy"
                                        .into(),
                                );
                                warn!(
                                    "RBM {path}: colour {diffuse}: requested resource/SRV={:?}, actual={:?}",
                                    view.selected_dxgi, image.texture_descriptor.format
                                );
                            }
                            debug!(
                                "RBM {path}: colour raw={} actual_resource/view={:?}",
                                diffuse, image.texture_descriptor.format
                            );
                            Some(cx.add_labeled_asset(format!("colour{i}"), image))
                        }
                        Err(error) => {
                            return Err(RbmLoadError::Diffuse {
                                raw_name: diffuse.clone(),
                                asset_path: view.asset_path.clone(),
                                message: error.to_string(),
                            });
                        }
                    };
                    colour_images.insert(key, (handle.clone(), view.clone()));
                    (handle, Some(view))
                }
            };
            let normal_map_texture = if let Some(name) = normal_texture_name(block, has_tangents) {
                let key = name.to_ascii_lowercase();
                if let Some(handle) = normal_images.get(&key) {
                    handle.clone()
                } else {
                    let tex_path = bevy::asset::AssetPath::from_path_buf(parent.join(name))
                        .with_source(source.clone());
                    // Settings do not participate in Bevy's deferred path-handle
                    // identity. Load a value and label it under this RBM, so even
                    // the same DDS used elsewhere as colour cannot alias this view.
                    let image = cx
                        .load_builder()
                        .with_settings(|settings: &mut ImageLoaderSettings| {
                            settings.is_srgb = false
                        })
                        .load_value::<Image>(tex_path)
                        .await;
                    let handle = match image {
                        Ok(image) => Some(cx.add_labeled_asset(format!("normal{i}"), image.take())),
                        Err(error) => {
                            warn!(
                                "RBM {}: normal {name} unavailable: {error}; using geometric normals",
                                path
                            );
                            None
                        }
                    };
                    normal_images.insert(key, handle.clone());
                    handle
                }
            } else {
                None
            };
            let material = StandardMaterial {
                base_color: if texture.is_some() {
                    Color::WHITE
                } else {
                    Color::srgb(0.7, 0.7, 0.75)
                },
                base_color_texture: texture,
                normal_map_texture,
                perceptual_roughness: if block.type_name.starts_with("CarPaint") {
                    0.35
                } else {
                    0.7
                },
                metallic: if block.type_name.starts_with("CarPaint") {
                    0.2
                } else {
                    0.0
                },
                alpha_mode: alpha_mode(&block.type_name),
                cull_mode: None,
                double_sided: true,
                ..default()
            };
            let foliage = block.foliage_flags.map(|flags| {
                let state = jc2_formats::foliage::FoliageAlpha::initialized(flags);
                let source = crate::foliage_alpha::source_material(material.clone(), state);
                debug!("RBM {path}: ordinary foliage flags={flags:#x} enabled={} instance_byte=255 reference={} distance_factor=1 (original scheduler unresolved086)",
                    state.enabled(),state.reference());
                RbmFoliage {material:cx.add_labeled_asset(format!("foliage{i}"),source),state}
            });
            if block.type_name == "BillboardFoliage" {
                warn!(
                    "RBM {path}: billboard x4 shader evidence exists, instance/enable ownership unsupported; retaining legacy alpha fallback"
                );
            }
            let material = cx.add_labeled_asset(format!("material{i}"), material);
            blocks.push(RbmBlock {
                type_name: block.type_name.clone(),
                diffuse,
                diffuse_view,
                foliage,
                mesh,
                material,
                triangles: block.mesh.triangle_count(),
                skinned: block.mesh.joints.len() == block.mesh.positions.len()
                    && !block.mesh.joints.is_empty(),
            });
        }
        debug!(
            "RBM {path}: original_colour_views={}/{} gamma_mode={:?}",
            blocks
                .iter()
                .filter(|block| block
                    .diffuse_view
                    .as_ref()
                    .is_some_and(|view| view.selected_dxgi.is_some()))
                .count(),
            blocks.len(),
            self.mode
        );
        Ok(RbmAsset {
            bbox_min: to_bevy(model.bbox_min).min(to_bevy(model.bbox_max)),
            bbox_max: to_bevy(model.bbox_min).max(to_bevy(model.bbox_max)),
            blocks,
            truncated_at: model.truncated_at.map(|(_, t)| t),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["rbm"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_source::JcAssetReader;
    use bevy::asset::io::{
        AssetReader, AssetReaderError, AssetSourceBuilder, PathStream, VecReader,
    };
    use bevy::asset::{AssetApp, AssetPlugin, AssetServer, Assets};
    use bevy::image::{CompressedImageFormats, ImageLoader};
    use jc2_formats::GameFs;
    use jc2_formats::rbm::{Material, MeshData};
    use std::path::Path;

    fn triangle_block() -> RenderBlock {
        RenderBlock {
            type_name: "General".into(),
            type_hash: jc2_formats::hash::hash_str("General"),
            version: 3,
            material: Material {
                diffuse: "test_dif.dds".into(),
                normal: "test_nrm.dds".into(),
                ..default()
            },
            mesh: MeshData {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                tangents: vec![
                    [2.0, 0.0, 0.02, 1.0],
                    [3.0, 0.0, -0.03, -1.0],
                    [1.0, 0.0, 0.0, 1.0],
                ],
                uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                uv1: vec![[0.2, 0.3], [0.5, 0.6], [0.8, 0.9]],
                indices: vec![0, 1, 2],
                ..default()
            },
            params: Vec::new(),
            foliage_flags: None,
            extra_attribute_streams: 0,
            instances: Vec::new(),
        }
    }

    #[test]
    fn foliage_masks_and_glass_blends_without_cutting_opaque_blocks() {
        for name in ["VegetationFoliage", "BillboardFoliage"] {
            assert_eq!(alpha_mode(name), AlphaMode::Mask(0.5));
        }
        for name in ["Window", "DeformableWindow"] {
            assert_eq!(alpha_mode(name), AlphaMode::Blend);
        }
        for name in [
            "VegetationBark",
            "General",
            "CarPaint",
            "Lambert",
            "SkinnedGeneral",
        ] {
            assert_eq!(alpha_mode(name), AlphaMode::Opaque);
        }
    }

    #[test]
    fn preserves_secondary_uv_and_signed_orthonormal_tangent_basis() {
        let block = triangle_block();
        let mesh = block_to_mesh(&block);
        let VertexAttributeValues::Float32x2(uv1) = mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap()
        else {
            panic!("secondary UV must be float2");
        };
        assert_eq!(uv1, &block.mesh.uv1);
        let VertexAttributeValues::Float32x4(tangents) =
            mesh.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap()
        else {
            panic!("tangent must be float4");
        };
        for (i, t) in tangents.iter().enumerate() {
            let tangent = Vec3::new(t[0], t[1], t[2]);
            assert!((tangent.length() - 1.0).abs() < 1e-6);
            assert!(tangent.dot(Vec3::Z).abs() < 1e-6);
            assert_eq!(t[3], if i == 1 { -1.0 } else { 1.0 });
            let bitangent = t[3] * Vec3::Z.cross(tangent);
            assert!(bitangent.dot(if i == 1 { -Vec3::Y } else { Vec3::Y }) > 0.999);
        }
    }

    #[test]
    fn invalid_basis_and_uv_streams_do_not_enable_normal_mapping() {
        for invalid in [
            [f32::NAN, 0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, f32::INFINITY],
        ] {
            let mut block = triangle_block();
            block.mesh.tangents[1] = invalid;
            assert!(!block_to_mesh(&block).contains_attribute(Mesh::ATTRIBUTE_TANGENT));
        }
        let mut block = triangle_block();
        block.mesh.tangents.pop();
        block.mesh.uv1[0][0] = f32::INFINITY;
        let mesh = block_to_mesh(&block);
        assert!(!mesh.contains_attribute(Mesh::ATTRIBUTE_TANGENT));
        assert!(!mesh.contains_attribute(Mesh::ATTRIBUTE_UV_1));
        assert!(normal_texture_name(&block, false).is_none());
    }

    #[test]
    fn normal_policy_rejects_placeholders_and_unverified_families() {
        let mut block = triangle_block();
        for name in ["General", "CarPaint", "VegetationBark"] {
            block.type_name = name.into();
            assert_eq!(normal_texture_name(&block, true), Some("test_nrm.dds"));
        }
        for name in [
            "VegetationFoliage",
            "BillboardFoliage",
            "SkinnedGeneral",
            "CarPaintSimple",
            "Window",
        ] {
            block.type_name = name.into();
            assert!(normal_texture_name(&block, true).is_none());
        }
        block.type_name = "General".into();
        block.material.normal = "TEST_DIF.DDS".into();
        assert!(normal_texture_name(&block, true).is_none());
        block.material.normal.clear();
        assert!(normal_texture_name(&block, true).is_none());
    }

    struct MissingNormals(JcAssetReader);

    #[derive(Clone, Copy)]
    enum DiffuseFault {
        Missing,
        Corrupt,
    }

    struct GammaFileOverride {
        reader: JcAssetReader,
        bytes: Option<Vec<u8>>,
        model: Option<Vec<u8>>,
        diffuse_fault: Option<DiffuseFault>,
        reads: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AssetReader for GammaFileOverride {
        async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
            if path.to_string_lossy().ends_with("mixed-colour.rbm") {
                return Ok(VecReader::new(self.model.clone().expect("fixture RBM")));
            }
            if path.to_string_lossy().ends_with("hook_diff.dds") {
                match self.diffuse_fault {
                    Some(DiffuseFault::Missing) => {
                        return Err(AssetReaderError::NotFound(path.to_path_buf()));
                    }
                    Some(DiffuseFault::Corrupt) => return Ok(VecReader::new(vec![0xff])),
                    None => {}
                }
            }
            if path.to_string_lossy().ends_with("gammacorrect.bin") {
                self.reads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return self
                    .bytes
                    .clone()
                    .map(VecReader::new)
                    .ok_or_else(|| AssetReaderError::NotFound(path.to_path_buf()));
            }
            let mut bytes = Vec::new();
            self.reader
                .read(path)
                .await?
                .read_to_end(&mut bytes)
                .await?;
            Ok(VecReader::new(bytes))
        }
        async fn read_meta<'a>(
            &'a self,
            path: &'a Path,
        ) -> Result<impl Reader + 'a, AssetReaderError> {
            self.reader.read_meta(path).await
        }
        async fn read_directory<'a>(
            &'a self,
            path: &'a Path,
        ) -> Result<Box<PathStream>, AssetReaderError> {
            self.reader.read_directory(path).await
        }
        async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
            self.reader.is_directory(path).await
        }
    }

    fn gamma_override(
        fs: Arc<GameFs>,
        bytes: Option<Vec<u8>>,
        reads: Arc<std::sync::atomic::AtomicUsize>,
    ) -> AssetSourceBuilder {
        AssetSourceBuilder::new(move || {
            Box::new(GammaFileOverride {
                reader: JcAssetReader::new(fs.clone()),
                bytes: bytes.clone(),
                model: None,
                diffuse_fault: None,
                reads: reads.clone(),
            })
        })
    }

    fn colour_fixture_reader(
        fs: Arc<GameFs>,
        model: Option<Vec<u8>>,
        fault: Option<DiffuseFault>,
    ) -> AssetSourceBuilder {
        let gamma = fs.read_inflated(GAMMA_CORRECT_FILE).unwrap();
        AssetSourceBuilder::new(move || {
            Box::new(GammaFileOverride {
                reader: JcAssetReader::new(fs.clone()),
                bytes: Some(gamma.clone()),
                model: model.clone(),
                diffuse_fault: fault,
                reads: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            })
        })
    }

    // Retain the retail vertex/index/material layouts. Only the two diffuse
    // strings, ordering and header block count change in these virtual assets.
    fn mixed_family_model(fs: &GameFs, verified_first: bool) -> Vec<u8> {
        let bytes = fs.read_inflated("lave.v046_buscoach.ee").unwrap();
        let archive = jc2_formats::sarc::SmallArchive::parse(&bytes).unwrap();
        let data = archive
            .data(&bytes, archive.get("v046-body_m_lod1.rbm").unwrap())
            .unwrap();
        let parsed = RbmModel::parse(data).unwrap();
        assert_eq!(parsed.blocks.len(), 4);
        assert_eq!(parsed.blocks[0].type_name, "CarPaint");
        assert_eq!(parsed.blocks[3].type_name, "DeformableWindow");
        let footer = jc2_formats::rbm::FOOTER.to_le_bytes();
        let ends: Vec<usize> = data
            .windows(4)
            .enumerate()
            .filter_map(|(i, bytes)| (bytes == footer).then_some(i + 4))
            .collect();
        assert_eq!(ends.len(), 4, "unique retail block footers");
        assert_eq!(ends[3], data.len());
        let replace_diffuse = |index: usize| {
            let begin = if index == 0 { 49 } else { ends[index - 1] };
            let block = &data[begin..ends[index]];
            let old = parsed.blocks[index].material.diffuse.as_bytes();
            let mut needle = (old.len() as u32).to_le_bytes().to_vec();
            needle.extend_from_slice(old);
            let offsets: Vec<usize> = block
                .windows(needle.len())
                .enumerate()
                .filter_map(|(i, bytes)| (bytes == needle).then_some(i))
                .collect();
            assert_eq!(offsets.len(), 1, "unique retail diffuse string");
            let offset = offsets[0];
            let name = b"hook_diff.dds";
            let mut output = block[..offset].to_vec();
            output.extend_from_slice(&(name.len() as u32).to_le_bytes());
            output.extend_from_slice(name);
            output.extend_from_slice(&block[offset + needle.len()..]);
            output
        };
        let verified = replace_diffuse(0);
        let unsupported = replace_diffuse(3);
        let mut output = data[..49].to_vec();
        output[45..49].copy_from_slice(&2u32.to_le_bytes());
        for block in if verified_first {
            [verified, unsupported]
        } else {
            [unsupported, verified]
        } {
            output.extend_from_slice(&block);
        }
        let fixture = RbmModel::parse(&output).unwrap();
        assert_eq!(fixture.blocks.len(), 2);
        assert!(
            fixture
                .blocks
                .iter()
                .all(|block| block.material.diffuse == "hook_diff.dds")
        );
        output
    }

    #[test]
    fn mixed_family_colour_cache_preserves_each_policy_in_both_block_orders() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        for verified_first in [true, false] {
            let model = mixed_family_model(&fs, verified_first);
            let mut app = app_with_reader(colour_fixture_reader(fs.clone(), Some(model), None));
            let handle: Handle<RbmAsset> = app
                .world()
                .resource::<AssetServer>()
                .load("jc2://general.blz/mixed-colour.rbm");
            wait_for_asset(&mut app, &handle);
            let models = app.world().resource::<Assets<RbmAsset>>();
            let blocks = &models.get(&handle).unwrap().blocks;
            let materials = app.world().resource::<Assets<StandardMaterial>>();
            let images = app.world().resource::<Assets<Image>>();
            let mut handles = Vec::new();
            for block in blocks {
                let verified = block.type_name == "CarPaint";
                let view = block.diffuse_view.as_ref().unwrap();
                assert_eq!(view.is_srgb, !verified, "order={verified_first}, {view:?}");
                assert_eq!(view.selected_dxgi, verified.then_some(71));
                if verified {
                    assert!(view.fallback.is_none());
                } else {
                    assert!(
                        view.fallback
                            .as_ref()
                            .unwrap()
                            .contains("unverified block family DeformableWindow")
                    );
                }
                let colour = materials
                    .get(&block.material)
                    .unwrap()
                    .base_color_texture
                    .as_ref()
                    .unwrap();
                assert_eq!(
                    images
                        .get(colour)
                        .unwrap()
                        .texture_descriptor
                        .format
                        .is_srgb(),
                    !verified
                );
                handles.push(colour.id());
            }
            assert_ne!(handles[0], handles[1]);
        }
    }

    #[test]
    fn missing_or_corrupt_requested_diffuse_fails_model_readiness() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        for fault in [DiffuseFault::Missing, DiffuseFault::Corrupt] {
            let mut app = app_with_reader(colour_fixture_reader(fs.clone(), None, Some(fault)));
            let handle: Handle<RbmAsset> = app
                .world()
                .resource::<AssetServer>()
                .load("jc2://general.blz/wea04_lod1-grapplinghook.rbm");
            let started = std::time::Instant::now();
            loop {
                app.update();
                assert!(
                    app.world()
                        .resource::<Assets<RbmAsset>>()
                        .get(&handle)
                        .is_none(),
                    "model must not become ready without its requested diffuse"
                );
                if let Some(bevy::asset::LoadState::Failed(error)) = app
                    .world()
                    .resource::<AssetServer>()
                    .get_load_state(handle.id())
                {
                    let message = error.to_string();
                    assert!(
                        message.contains("required diffuse hook_diff.dds"),
                        "{message}"
                    );
                    assert!(
                        message
                            .replace('\\', "/")
                            .contains("general.blz/hook_diff.dds"),
                        "{message}"
                    );
                    break;
                }
                assert!(started.elapsed().as_secs() < 60, "expected diffuse failure");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    fn hook_exception_file() -> Vec<u8> {
        use jc2_formats::property::{Key, Node, PropertyFile, Variant};
        PropertyFile {
            nodes: vec![Node {
                children: vec![(
                    Key::Name("root".into()),
                    Node {
                        values: vec![(Key::Hash(1), Variant::Str("HOOK_DIFF.dds".into()))],
                        ..default()
                    },
                )],
                ..default()
            }],
        }
        .to_bytes()
    }

    impl AssetReader for MissingNormals {
        async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
            if path.to_string_lossy().ends_with("_nrm.dds") {
                return Err(AssetReaderError::NotFound(path.to_path_buf()));
            }
            let mut bytes = Vec::new();
            self.0.read(path).await?.read_to_end(&mut bytes).await?;
            Ok(VecReader::new(bytes))
        }

        async fn read_meta<'a>(
            &'a self,
            path: &'a Path,
        ) -> Result<impl Reader + 'a, AssetReaderError> {
            self.0.read_meta(path).await
        }

        async fn read_directory<'a>(
            &'a self,
            path: &'a Path,
        ) -> Result<Box<PathStream>, AssetReaderError> {
            self.0.read_directory(path).await
        }

        async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
            self.0.is_directory(path).await
        }
    }

    fn loader_app(fs: Arc<GameFs>) -> App {
        app_with_reader(AssetSourceBuilder::new(move || {
            Box::new(JcAssetReader::new(fs.clone()))
        }))
    }

    fn app_with_reader(reader: AssetSourceBuilder) -> App {
        app_with_reader_and_loader(reader, RbmLoader::default())
    }

    fn app_with_reader_and_loader(reader: AssetSourceBuilder, loader: RbmLoader) -> App {
        let mut app = App::new();
        app.register_asset_source("jc2", reader)
            .add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<Image>()
            .init_asset::<RbmAsset>()
            .init_asset::<crate::foliage_alpha::FoliageMaterial>()
            // Headless loading retains original BC pixels; no GPU is needed to
            // inspect the colour/data views. DDS does not decompress BC on an
            // unsupported device, so NONE would reject the retail maps.
            .register_asset_loader(ImageLoader::new(CompressedImageFormats::BC))
            .register_asset_loader(loader);
        app
    }

    #[test]
    fn missing_retail_normal_keeps_model_and_geometric_material_fallback() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        let mut app = app_with_reader(AssetSourceBuilder::new(move || {
            Box::new(MissingNormals(JcAssetReader::new(fs.clone())))
        }));
        let handle: Handle<RbmAsset> = app
            .world()
            .resource::<AssetServer>()
            .load("jc2://vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm");
        wait_for_asset(&mut app, &handle);
        let model = app
            .world()
            .resource::<Assets<RbmAsset>>()
            .get(&handle)
            .unwrap();
        assert_eq!(model.blocks.len(), 2);
        assert_eq!(
            model.blocks.iter().map(|b| b.triangles).sum::<usize>(),
            2851
        );
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let bark = model
            .blocks
            .iter()
            .find(|b| b.type_name == "VegetationBark")
            .unwrap();
        let material = materials.get(&bark.material).unwrap();
        assert!(material.normal_map_texture.is_none());
        assert!(material.base_color_texture.is_some());
        assert_eq!(material.alpha_mode, AlphaMode::Opaque);
    }

    fn wait_for_asset<A: Asset>(app: &mut App, handle: &Handle<A>) {
        let started = std::time::Instant::now();
        while started.elapsed().as_secs() < 60 {
            app.update();
            if app.world().resource::<Assets<A>>().get(handle).is_some() {
                return;
            }
            if let Some(bevy::asset::LoadState::Failed(error)) = app
                .world()
                .resource::<AssetServer>()
                .get_load_state(handle.id())
            {
                panic!("asset failed: {handle:?}: {error}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("asset did not load: {handle:?}");
    }

    #[test]
    fn original_name_only_mode_does_not_read_authored_exceptions() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut app = app_with_reader_and_loader(
            gamma_override(fs, Some(hook_exception_file()), reads.clone()),
            RbmLoader::new(GammaMode::OnlyTextureNames),
        );
        let hook: Handle<RbmAsset> = app
            .world()
            .resource::<AssetServer>()
            .load("jc2://general.blz/wea04_lod1-grapplinghook.rbm");
        wait_for_asset(&mut app, &hook);
        assert_eq!(reads.load(std::sync::atomic::Ordering::Relaxed), 0);
        let models = app.world().resource::<Assets<RbmAsset>>();
        let view = models.get(&hook).unwrap().blocks[0]
            .diffuse_view
            .as_ref()
            .unwrap();
        assert_eq!(view.selected_dxgi, Some(71));
        assert!(view.fallback.is_none());
    }

    #[test]
    fn gamma_cache_separates_asset_sources_and_reuses_mounted_policy() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        let original = fs.read_inflated(GAMMA_CORRECT_FILE).unwrap();
        let one_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let two_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut app = App::new();
        app.register_asset_source(
            "one",
            gamma_override(fs.clone(), Some(hook_exception_file()), one_reads.clone()),
        )
        .register_asset_source(
            "two",
            gamma_override(fs.clone(), Some(original), two_reads.clone()),
        )
        .add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .init_asset::<RbmAsset>()
        .init_asset::<crate::foliage_alpha::FoliageMaterial>()
        .register_asset_loader(ImageLoader::new(CompressedImageFormats::BC))
        .register_asset_loader(RbmLoader::default());
        let one: Handle<RbmAsset> = app
            .world()
            .resource::<AssetServer>()
            .load("one://general.blz/wea04_lod1-grapplinghook.rbm");
        wait_for_asset(&mut app, &one);
        let two: Handle<RbmAsset> = app
            .world()
            .resource::<AssetServer>()
            .load("two://general.blz/wea04_lod1-grapplinghook.rbm");
        wait_for_asset(&mut app, &two);
        // A later model on each source must not reread the authored table.
        for source in ["one", "two"] {
            let jeep: Handle<RbmAsset> = app
                .world()
                .resource::<AssetServer>()
                .load(format!("{source}://lave.v014_jeep.ee/v014_lod1-cover.rbm"));
            wait_for_asset(&mut app, &jeep);
        }
        assert_eq!(one_reads.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(two_reads.load(std::sync::atomic::Ordering::Relaxed), 1);
        let models = app.world().resource::<Assets<RbmAsset>>();
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let images = app.world().resource::<Assets<Image>>();
        for (handle, srgb) in [(&one, true), (&two, false)] {
            let block = &models.get(handle).unwrap().blocks[0];
            let material = materials.get(&block.material).unwrap();
            let image = images
                .get(material.base_color_texture.as_ref().unwrap())
                .unwrap();
            assert_eq!(image.texture_descriptor.format.is_srgb(), srgb);
            assert_eq!(
                block.diffuse_view.as_ref().unwrap().selected_dxgi,
                Some(if srgb { 72 } else { 71 })
            );
        }
    }

    #[test]
    fn missing_or_malformed_gamma_asset_has_explicit_name_only_fallback() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        for bytes in [None, Some(vec![0xff])] {
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut app = app_with_reader(gamma_override(fs.clone(), bytes, reads));
            let hook: Handle<RbmAsset> = app
                .world()
                .resource::<AssetServer>()
                .load("jc2://general.blz/wea04_lod1-grapplinghook.rbm");
            wait_for_asset(&mut app, &hook);
            let models = app.world().resource::<Assets<RbmAsset>>();
            let block = &models.get(&hook).unwrap().blocks[0];
            let view = block.diffuse_view.as_ref().unwrap();
            assert_eq!(view.selected_dxgi, Some(71));
            assert!(
                view.fallback
                    .as_ref()
                    .unwrap()
                    .contains("name-only fallback")
            );
            let materials = app.world().resource::<Assets<StandardMaterial>>();
            let images = app.world().resource::<Assets<Image>>();
            let material = materials.get(&block.material).unwrap();
            assert!(
                !images
                    .get(material.base_color_texture.as_ref().unwrap())
                    .unwrap()
                    .texture_descriptor
                    .format
                    .is_srgb()
            );
        }
    }

    #[test]
    fn retail_hook_jeep_and_pine_use_original_independent_colour_views() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let mut app = loader_app(Arc::new(GameFs::open(dir).unwrap()));
        let asset_server = app.world().resource::<AssetServer>();
        let hook: Handle<RbmAsset> =
            asset_server.load("jc2://general.blz/wea04_lod1-grapplinghook.rbm");
        let jeep: Handle<RbmAsset> =
            asset_server.load("jc2://lave.v014_jeep.ee/v014_lod1-cover.rbm");
        let pine: Handle<RbmAsset> =
            asset_server.load("jc2://vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm");
        // Loading this exact path first as default colour must not force the
        // original hook's independently selected linear view to become sRGB.
        let default_hook: Handle<Image> = asset_server.load("jc2://general.blz/hook_diff.dds");
        wait_for_asset(&mut app, &default_hook);
        for model in [&hook, &jeep, &pine] {
            wait_for_asset(&mut app, model);
        }
        let models = app.world().resource::<Assets<RbmAsset>>();
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let images = app.world().resource::<Assets<Image>>();
        for (model, type_name, raw_name, source, selected, srgb) in [
            (&hook, "General", "hook_diff.dds", 71, 71, false),
            (&jeep, "CarPaint", "v014_body_dif.dds", 71, 72, true),
            (
                &pine,
                "VegetationBark",
                "arctic_pine_bark_dif.dds",
                71,
                72,
                true,
            ),
            (
                &pine,
                "VegetationFoliage",
                "arctic_pine_foliage_dif.dds",
                77,
                78,
                true,
            ),
        ] {
            let block = models
                .get(model)
                .unwrap()
                .blocks
                .iter()
                .find(|block| block.type_name == type_name)
                .unwrap();
            let view = block.diffuse_view.as_ref().unwrap();
            assert_eq!(view.raw_name, raw_name);
            assert!(view.asset_path.starts_with("jc2://"));
            assert_eq!(
                (view.source_dxgi, view.selected_dxgi),
                (Some(source), Some(selected))
            );
            assert!(view.fallback.is_none(), "{view:?}");
            let material = materials.get(&block.material).unwrap();
            let colour_handle = material.base_color_texture.as_ref().unwrap();
            let colour = images.get(colour_handle).unwrap();
            assert_eq!(
                colour.texture_descriptor.format.is_srgb(),
                srgb,
                "{raw_name}"
            );
            if let Some(normal_handle) = &material.normal_map_texture {
                assert_ne!(colour_handle.id(), normal_handle.id());
                assert!(
                    !images
                        .get(normal_handle)
                        .unwrap()
                        .texture_descriptor
                        .format
                        .is_srgb()
                );
            }
            if model.id() == hook.id() {
                assert_ne!(colour_handle.id(), default_hook.id());
                let default_image = images.get(&default_hook).unwrap();
                assert!(default_image.texture_descriptor.format.is_srgb());
                assert_eq!(colour.data, default_image.data);
                assert_eq!(
                    colour.texture_descriptor.size,
                    default_image.texture_descriptor.size
                );
            }
        }
    }

    #[test]
    fn retail_pine_masks_leaves_and_keeps_normal_images_linear_and_distinct() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let mut app = loader_app(Arc::new(GameFs::open(dir).unwrap()));
        let model: Handle<RbmAsset> = app
            .world()
            .resource::<AssetServer>()
            .load("jc2://vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm");
        // Deliberately load the same normal file independently with colour
        // defaults. This catches settings aliasing through a path-based handle.
        let colour_view: Handle<Image> = app
            .world()
            .resource::<AssetServer>()
            .load("jc2://vegetation_1.blz/arctic_pine_bark_nrm.dds");
        wait_for_asset(&mut app, &model);
        wait_for_asset(&mut app, &colour_view);
        let model = app
            .world()
            .resource::<Assets<RbmAsset>>()
            .get(&model)
            .unwrap();
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let meshes = app.world().resource::<Assets<Mesh>>();
        let mut normal_view = None;
        for block in &model.blocks {
            let material = materials.get(&block.material).unwrap();
            let mesh = meshes.get(&block.mesh).unwrap();
            assert!(mesh.contains_attribute(Mesh::ATTRIBUTE_UV_1));
            assert!(mesh.contains_attribute(Mesh::ATTRIBUTE_TANGENT));
            if block.type_name == "VegetationFoliage" {
                assert_eq!(material.alpha_mode, AlphaMode::Mask(0.5));
                assert!(material.normal_map_texture.is_none());
                let leaf = block
                    .foliage
                    .as_ref()
                    .expect("ordinary foliage must use its native alpha route");
                assert_eq!(
                    leaf.state,
                    jc2_formats::foliage::FoliageAlpha::initialized(0x40)
                );
                let custom = app
                    .world()
                    .resource::<Assets<crate::foliage_alpha::FoliageMaterial>>()
                    .get(&leaf.material)
                    .unwrap();
                assert_eq!(custom.extension.state, leaf.state);
                assert_eq!(custom.base.alpha_mode, AlphaMode::Mask(0.0));
                assert_eq!(
                    custom.base.opaque_render_method,
                    bevy::material::OpaqueRendererMethod::Forward
                );
                assert!(custom.base.normal_map_texture.is_none());
                assert_eq!(custom.base.base_color_texture, material.base_color_texture);
            } else {
                assert_eq!(material.alpha_mode, AlphaMode::Opaque);
                normal_view = material.normal_map_texture.clone();
            }
        }
        let normal_view = normal_view.expect("retail bark normal must load");
        assert_ne!(normal_view.id(), colour_view.id());
        let images = app.world().resource::<Assets<Image>>();
        let normal = images.get(&normal_view).unwrap();
        let colour = images.get(&colour_view).unwrap();
        assert!(!normal.texture_descriptor.format.is_srgb());
        assert!(colour.texture_descriptor.format.is_srgb());
        assert_eq!(normal.data, colour.data);
    }

    #[test]
    fn retail_bark_signed_basis_matches_its_authored_bitangent_stream() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let bytes = fs.read_inflated("vegetation_1.blz").unwrap();
        let sarc = jc2_formats::sarc::SmallArchive::parse(&bytes).unwrap();
        let entry = sarc.get("Arctic_T22_PineL_LOD1-Whole.rbm").unwrap();
        let data = sarc.data(&bytes, entry).unwrap();
        let model = RbmModel::parse(data).unwrap();
        let block = &model.blocks[0];
        assert_eq!(block.type_name, "VegetationBark");
        let tangents = mesh_tangents(block, &block.mesh.normals).unwrap();
        // Retail first block: RBMDL header 49 bytes, type hash, v0, flags,
        // eight texture strings + material trailer, vertex count, 40-byte verts.
        let mut reader = jc2_formats::reader::Reader::new(data);
        reader.seek(53).unwrap();
        assert_eq!(reader.u8().unwrap(), 0);
        assert_eq!(reader.u32().unwrap(), 0);
        for _ in 0..8 {
            let len = reader.u32().unwrap() as usize;
            reader.bytes(len).unwrap();
        }
        reader.u32().unwrap();
        let count = reader.u32().unwrap() as usize;
        assert_eq!(count, tangents.len());
        let mut agreement = 0.0;
        for (i, t) in tangents.iter().enumerate() {
            reader.bytes(36).unwrap();
            let (bitangent, _) = jc2_formats::rbm::unpack_normal(reader.f32().unwrap());
            let n = Vec3::from(block.mesh.normals[i]).normalize();
            let expected = t[3] * n.cross(Vec3::new(t[0], t[1], t[2]));
            agreement += expected.dot(Vec3::from(bitangent).normalize());
        }
        // A few authored vertices have inconsistent unused bitangents; the
        // original bark shader reconstructs the basis using the packed sign.
        assert!(
            agreement / count as f32 > 0.95,
            "basis agreement {}",
            agreement / count as f32
        );
    }

    /// Headless: drive the asset server without a renderer and make sure the
    /// loader produces meshes/materials from a model inside a container.
    /// Needs JC2_TEST_DIR (a game folder subset with pc4.* is enough).
    #[test]
    fn loads_bus_body_through_asset_server() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        let mut app = loader_app(fs);
        let handle: Handle<RbmAsset> = app
            .world()
            .resource::<AssetServer>()
            .load("jc2://lave.v046_buscoach.ee/v046-body_m_lod1.rbm");
        let mut loaded = None;
        let started = std::time::Instant::now();
        while started.elapsed().as_secs() < 60 {
            app.update();
            if let Some(a) = app.world().resource::<Assets<RbmAsset>>().get(&handle) {
                loaded = Some((
                    a.blocks.len(),
                    a.blocks.iter().map(|b| b.triangles).sum::<usize>(),
                    a.bbox_min,
                    a.bbox_max,
                ));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (blocks, tris, lo, hi) = loaded.expect("rbm asset did not load");
        assert_eq!(blocks, 4);
        assert_eq!(tris, 4380);
        assert!(lo.x < -1.5 && hi.x > 1.5 && hi.y > 4.0, "{lo:?} {hi:?}");
        // the long axis (z) is ~13.7 m
        assert!((hi.z - lo.z) > 13.0);
        let meshes = app.world().resource::<Assets<Mesh>>();
        assert!(meshes.len() >= 4);
        let asset = app
            .world()
            .resource::<Assets<RbmAsset>>()
            .get(&handle)
            .unwrap();
        let materials = app.world().resource::<Assets<StandardMaterial>>();
        let images = app.world().resource::<Assets<Image>>();
        let mut normal_count = 0;
        for block in &asset.blocks {
            let material = materials.get(&block.material).unwrap();
            if block.type_name == "DeformableWindow" {
                assert_eq!(material.alpha_mode, AlphaMode::Blend);
                assert!(material.normal_map_texture.is_none());
            } else {
                assert_eq!(block.type_name, "CarPaint");
                assert_eq!(material.alpha_mode, AlphaMode::Opaque);
                assert!(
                    meshes
                        .get(&block.mesh)
                        .unwrap()
                        .contains_attribute(Mesh::ATTRIBUTE_TANGENT)
                );
                if let Some(normal) = &material.normal_map_texture {
                    assert!(
                        !images
                            .get(normal)
                            .unwrap()
                            .texture_descriptor
                            .format
                            .is_srgb()
                    );
                    normal_count += 1;
                }
            }
        }
        assert!(
            normal_count > 0,
            "bus fixture must exercise real normal loading"
        );
    }
}
