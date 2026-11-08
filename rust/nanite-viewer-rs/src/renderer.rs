//! The wgpu port of `examples/pbrtexture/pbrtexture.cpp`.
//!
//! Everything the C++ example creates in [`prepare`] order and records in
//! [`buildCommandBuffers`] lives here, in the same order, with the same buffer sizes and the same
//! pipeline state. The C++ function names are quoted throughout so a diff against the original is
//! possible.
//!
//! The frame graph, per frame (`draw()` + `buildCommandBuffers()`):
//!
//! ```text
//!   copy initNodeInfos -> currNodeInfos          (reset the traversal frontier)
//!   clear culledClusterIndices[0..5]             (4 atomic counters + list head)
//!   for depth level j: bvhtraversal[j & 1]       (ping-pong curr/next node lists)
//!   error                                            (cluster error -> screen radius^2)
//!   culling                                          (LOD gate + frustum + HiZ occlusion)
//!   clearimage                                       (zero the 64-bit visibility image,
//!                                                     and compute the swr indirect dispatch)
//!   swrasterize            (indirect)                (software rasterizer)
//!   ---- render pass: hwrasterize (non-indexed indirect draw)
//!   merger                                           (merge both visibility buffers)
//!   ---- render pass: skybox (optional) + shading
//!   depthcopy                                        (final depth -> HiZ mip 0)
//!   for mip j: genhiz[j]                             (build the HiZ pyramid)
//! ```
//!
//! Four places where wgpu forces a different shape than Vulkan, all documented at the code:
//!
//! 1. `hwrasterize` is a **non-indexed** draw: WGSL has no geometry shader, so `culling.wgsl`
//!    expands the triangles itself and writes a packed id per output vertex
//!    ([`shaders::HWRASTERIZE`]'s header comment).
//! 2. The hw-raster pass has a **second, `R32Float` colour attachment** ([`Self::hwr_depth_copy`]):
//!    wgpu 30 refuses to bind a view of a depth image at all, so `merger.comp`'s
//!    `textureLoad(HWRZBuffer, ..)` is replaced by the fragment stage writing `fragCoord.z` to a
//!    colour attachment the merger can read.
//! 3. The two host-side counter resets `draw()` does with `vkCmdUpdateBuffer` + `vkDeviceWaitIdle`
//!    become [`wgpu::CommandEncoder::clear_buffer`] at the head of the frame.
//! 4. There is **no HiZ sampler at all**. `bvhtraversal.wgsl` and `culling.wgsl` tap `lastHZB` with
//!    `textureLoad` ([`shaders::COMMON`]'s `occlusionCulling`), because a WGSL `sampler` is always a
//!    *filtering* one and wgpu then rejects the non-filterable `R32Float` HiZ view unless
//!    [`wgpu::Features::FLOAT32_FILTERABLE`] is enabled. The C++ `hizbuffer.sampler` is
//!    `VK_FILTER_NEAREST` + `CLAMP_TO_EDGE`, so `floor(uv * size)` clamped into the mip is the same
//!    four texels - and nothing in this crate depends on `FLOAT32_FILTERABLE`.

use anyhow::{bail, Result};
use glam::{Mat4, Vec3};

use crate::{abi, camera::Camera, env, shaders};

/// `WORKGROUP_SIZE` in `shaders/glsl/pbrtexture/*.comp` for the 1D compute shaders.
const WORKGROUP_SIZE_1D: u32 = 32;
/// `WORKGROUP_SIZE` for the image-sized compute shaders (`clearimage`, `merger`, `depthcopy`,
/// `genhiz`) - `vulkanexamplebase.h:154`.
const WORKGROUP_SIZE_2D: u32 = 8;

/// The features the nanite pipeline asks for. `SHADER_INT64` + the 64-bit image atomic are what the
/// visibility buffer needs; `IMMEDIATES` is what every push-constant block became.
///
/// Callers must intersect this with [`wgpu::Adapter::features`] - the crate deliberately does not
/// pretend a driver without 64-bit image atomics can run the pipeline.
pub const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::IMMEDIATES
    .union(wgpu::Features::SHADER_INT64)
    .union(wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX)
    .union(wgpu::Features::TEXTURE_INT64_ATOMIC)
    .union(wgpu::Features::TEXTURE_ATOMIC);

/// The main pass' depth attachment format, i.e. the `depthStencil` image of the C++ example.
///
/// Public because a capture renders into its own depth buffer ([`crate::capture::CaptureTarget`]) and
/// both must be the same format, or the captured frame is not the presented frame.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// A depth state that **declares the attachment but disables the test and the write**.
///
/// wgpu 30 compares a pipeline's depth-stencil format with the render pass' attachment as
/// `Option<TextureFormat>` (`wgpu-core-30.0.1/src/device/mod.rs:147`), so a pipeline used in a pass
/// that has a depth buffer must name that format even when the C++ original sets
/// `depthStencilState.depthTestEnable = VK_FALSE` (`examples/pbrtexture/pbrtexture.cpp:2006`).
/// Leaving `depth_stencil: None` there fails with "Incompatible depth-stencil attachment format:
/// the RenderPass uses a texture with format Some(Depth32Float) but the RenderPipeline ... uses an
/// attachment with format None".
///
/// `depth_write_enabled: Some(false)` makes `depth_compare: None` legal
/// (`wgpu-core-30.0.1/src/device/resource.rs:4545`: the compare function is only required when
/// writing or when a stencil face reacts to a depth failure), which is exactly "no depth operation".
fn depth_state_off() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: None,
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// `thresholdIntDiv` in `pbrtexture.cpp`: the *Visualize/Threshold* slider is an integer 0..1000 and
/// the shader wants world units, so the slider value is scaled by this.
pub const THRESHOLD_INT_DIV: f32 = 1e-6;

/// The subset of `VulkanExampleBase`'s state the frame loop feeds the renderer.
#[derive(Clone, Copy, Debug)]
pub struct FrameSettings {
    /// `thresholdInt * thresholdIntDiv`, i.e. `500 * 1e-6` by default.
    pub threshold: f32,
    /// `useFrustrumOcclusionCulling`.
    pub frustum_occlusion: bool,
    /// `useSoftwareRast`.
    pub software_raster: bool,
    /// `displaySkybox`.
    pub skybox: bool,
    /// `renderingPushConstants.vis_clusters`, 0..3.
    pub visualize: i32,
    /// `uniformBuffers.params` exposure.
    pub exposure: f32,
    /// `uniformBuffers.params` gamma.
    pub gamma: f32,
}

impl Default for FrameSettings {
    fn default() -> Self {
        Self {
            // `thresholdInt = 500`, `thresholdIntDiv = 1e-6f`.
            threshold: 500.0 * 1e-6,
            frustum_occlusion: true,
            software_raster: true,
            skybox: true,
            visualize: 0,
            exposure: 4.5,
            gamma: 2.2,
        }
    }
}

/// What one frame rasterized, mirroring the four overlay lines of `OnUpdateUIOverlay`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RasterStats {
    /// `scene.sceneIndicesCount` - the upper bound on emitted triangles.
    pub scene_indices: u32,
    /// `hwrDrawIndexedIndirectBuffer.indexCount`.
    pub hw_indices: u32,
    /// `swrNumVerticesBuffer.indexCount`.
    pub sw_indices: u32,
}

impl RasterStats {
    /// Triangles, i.e. the three `/ 3` lines of the C++ overlay.
    pub fn hw_triangles(&self) -> u32 {
        self.hw_indices / 3
    }
    pub fn sw_triangles(&self) -> u32 {
        self.sw_indices / 3
    }
    pub fn total_triangles(&self) -> u32 {
        (self.sw_indices + self.hw_indices) / 3
    }
    pub fn scene_triangles(&self) -> u32 {
        self.scene_indices / 3
    }
}

/// The scene buffers that have to be replaced when a model is loaded or reloaded
/// (`freeNaniteBuffers` + `createBVHTraversalBuffers` + `createCullingBuffers` +
/// `createErrorProjectionBuffer` + `createModelMatsBuffer`).
///
/// Everything else - pipelines, rasters, HiZ, env maps, UBOs - survives a reload, exactly as in
/// `reloadModel()`, which deliberately skips the one-time setup.
struct NaniteBuffers {
    /// `vertexBuffer` of the scene (`createVertexIndexBuffer`): the concatenated `vkglTF::Vertex` array.
    scene_vertices: wgpu::Buffer,
    /// `indexBuffer` of the scene, already offset per mesh.
    scene_indices: wgpu::Buffer,
    bvh_node_infos: wgpu::Buffer,
    init_node_infos: wgpu::Buffer,
    curr_node_infos: wgpu::Buffer,
    next_node_infos: wgpu::Buffer,
    sorted_cluster_indices: wgpu::Buffer,
    culled_cluster_indices: wgpu::Buffer,
    culled_cluster_object_indices: wgpu::Buffer,
    hw_r_indices: wgpu::Buffer,
    hw_r_id: wgpu::Buffer,
    /// `HWRVertexIdsBuffer`: the packed visibility id `culling.comp` writes per *output* hw vertex,
    /// read back by `hwrasterize.wgsl`'s vertex stage. Replaces `hwrasterize.geom`'s per-fragment id.
    hw_r_vertex_ids: wgpu::Buffer,
    sw_r_indices: wgpu::Buffer,
    sw_r_id: wgpu::Buffer,
    clusters_info: wgpu::Buffer,
    error_info: wgpu::Buffer,
    projected_error: wgpu::Buffer,
    model_mats: wgpu::Buffer,
    /// `[count, 0, 1, ...]`, the source of the per-frame copy into `curr_node_infos`.
    init_node_info_bytes: u64,
    depth_counts: Vec<u32>,
    max_cluster_num: u32,
    scene_indices_count: u32,
}

/// The per-model bind groups (`writeDescriptors`), rebuilt together with [`NaniteBuffers`].
struct NaniteBindGroups {
    bvh_traversal: [wgpu::BindGroup; 2],
    error_proj: wgpu::BindGroup,
    culling: wgpu::BindGroup,
    sw_rast: wgpu::BindGroup,
    hw_rast: wgpu::BindGroup,
    shading: wgpu::BindGroup,
}

/// The screen-sized buffers and images (`createRasterizeBuffer`, `createHWRasterizeFramebuffer`,
/// `createHiZBuffer`, `setupDepthStencil`). Recreated when the window is resized.
/// The screen-sized images of one frame (`createRasterizeBuffer` + `setupRenderPass`'s framebuffer +
/// `createHiZBuffer` + `setupDepthStencil`). The bare `wgpu::Texture` fields are never *read*: they
/// exist so the views handed to bind groups and render passes stay alive as long as this struct.
#[allow(dead_code)]
struct RasterTargets {
    hw_r_vis: wgpu::Texture,
    hw_r_vis_view: wgpu::TextureView,
    /// The `R32Float` stand-in for `HWRZBuffer` - see the module docs.
    hwr_depth_copy: wgpu::Texture,
    hwr_depth_copy_view: wgpu::TextureView,
    sw_r: wgpu::Texture,
    sw_r_view: wgpu::TextureView,
    final_vis: wgpu::Texture,
    final_vis_view: wgpu::TextureView,
    final_z: wgpu::Texture,
    final_z_view: wgpu::TextureView,
    hiz: wgpu::Texture,
    /// One view per mip, `hizImageViews` in C++.
    hiz_views: Vec<wgpu::TextureView>,
    hiz_mip_levels: u32,
    /// The main pass' depth attachment (`depthStencil`), `Depth32Float`.
    depth_stencil: wgpu::Texture,
    depth_stencil_view: wgpu::TextureView,
    width: u32,
    height: u32,
}

/// The renderer: pipelines, buffers, bind groups and the frame recording.
///
/// Like the C++ example it owns nothing related to a window; the caller hands it a colour and
/// depth view per frame ([`Renderer::render`]), which is what lets the same code drive a swapchain
/// and an offscreen capture target.
pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub features: wgpu::Features,

    // ---- pipelines (preparePipelines) ----
    bvh_traversal: wgpu::ComputePipeline,
    error: wgpu::ComputePipeline,
    culling: wgpu::ComputePipeline,
    sw_rasterize: wgpu::ComputePipeline,
    clear_image: wgpu::ComputePipeline,
    merger: wgpu::ComputePipeline,
    hw_rasterize: wgpu::RenderPipeline,
    shading: wgpu::RenderPipeline,
    skybox: wgpu::RenderPipeline,
    depth_copy: wgpu::ComputePipeline,
    gen_hiz: wgpu::ComputePipeline,

    // ---- bind group layouts, taken off the derived pipelines ----
    bvh_traversal_bgl: wgpu::BindGroupLayout,
    error_bgl: wgpu::BindGroupLayout,
    culling_bgl: wgpu::BindGroupLayout,
    sw_rast_bgl: wgpu::BindGroupLayout,
    clear_image_bgl: wgpu::BindGroupLayout,
    merger_bgl: wgpu::BindGroupLayout,
    hw_rast_bgl: wgpu::BindGroupLayout,
    shading_bgl: wgpu::BindGroupLayout,
    depth_copy_bgl: wgpu::BindGroupLayout,
    /// The one hand-written layout: `hizBuild` binds two *different* mips of one texture, which a
    /// derived layout cannot express per-instance. (It can - the layout is per pipeline, the groups
    /// per binding - but keeping the layout explicit makes the two-view groups obvious.)
    hiz_build_bgl: wgpu::BindGroupLayout,

    // ---- uniform buffers (prepareUniformBuffers) ----
    ubo_object: wgpu::Buffer,
    ubo_skybox: wgpu::Buffer,
    ubo_params: wgpu::Buffer,
    ubo_culling: wgpu::Buffer,
    ubo_error: wgpu::Buffer,
    ubo_shading: wgpu::Buffer,

    // ---- raster buffers that outlive a model (createRasterizeBuffer) ----
    hwr_draw_indirect: wgpu::Buffer,
    swr_indirect_dispatch: wgpu::Buffer,
    swr_num_vertices: wgpu::Buffer,

    // ---- samplers ----
    env_sampler: wgpu::Sampler,

    // ---- static scene objects ----
    cube: env::CubeMesh,
    skybox_bind_group: wgpu::BindGroup,

    env_maps: env::EnvMaps,
    targets: RasterTargets,
    nanite: Option<(NaniteBuffers, NaniteBindGroups)>,

    /// `uboCullingMatrices.model` - **not** identity, see `createCullingBuffers`.
    culling_model: Mat4,
    /// `uboMatrices.model` for the object pipelines: `createScene2()`'s `modelMats[0]`, i.e.
    /// `rotateY(-90) * translate(0,0,3)`. Bound by `hwrasterize` (binding 2) and `swrasterize`
    /// (binding 6) as `uniformBuffers.object`.
    object_model: Mat4,
    /// The last scene handed to [`Renderer::load_scene`], kept so [`Renderer::resize`] can rebuild
    /// the scene buffers against the new raster targets (`createRasterizeBuffer` is part of what a
    /// resize tears down, and the bind groups reference those views).
    loaded_scene: Option<crate::scene::Scene>,
    /// `uboCullingMatrices.lastView/lastProj`, updated after the frame was submitted.
    last_view: Mat4,
    last_proj: Mat4,

    width: u32,
    height: u32,
    /// The `--raster-stats` counter, mirroring `VulkanExampleBase::framesRendered`.
    frame_index: u64,
}

impl Renderer {
    /// Build everything except the nanite scene buffers, in the C++ `prepare()` order:
    /// `createRasterizeBuffer` -> `createHWRasterizeFramebuffer` -> `createHiZBuffer` ->
    /// `prepareUniformBuffers` -> `preparePipelines`, with the environment-map generation of
    /// `generateBRDFLUT`/`generateIrradianceCube`/`generatePrefilteredCube` folded into
    /// [`env::EnvMaps::new`].
    ///
    /// `surface_format` is only used to size the main pass' depth attachment to what will be
    /// rendered; the colour format itself never reaches this type.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        ktx_path: &std::path::Path,
    ) -> Result<Self> {
        let features = device.features();
        let missing: Vec<String> = REQUIRED_FEATURES
            .intersection(!features)
            .iter_names()
            .map(|(name, _)| name.to_string())
            .collect();
        if !missing.is_empty() {
            bail!(
                "the nanite pipeline needs GPU features this device lacks: {}",
                missing.join(", ")
            );
        }

        let modules = shaders::create_modules(&device);

        // createRasterizeBuffer: the two indirect argument buffers and the software rasterizer's
        // vertex counter. In C++ the last two are host-visible because the host zeroes them between
        // frames; here the frame encoder clears them instead, so they are device-local.
        let hwr_draw_indirect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hwrDrawIndexedIndirect"),
            size: abi::SIZE_HWR_DRAW_INDIRECT as u64,
            usage: wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let swr_indirect_dispatch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swrIndirectDispatch"),
            size: abi::SIZE_SWR_DISPATCH as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        // `swrNumVerticesBuffer` is one buffer with three bindings in C++: a storage atomic counter
        // for `culling.comp`, a storage read for `clearimage.comp`, and the *uniform* `DrawInfo` of
        // `swrasterize.comp`. Vulkan tolerates a 4-byte uniform range; WGSL's `var<uniform>` must
        // cover the whole `DrawInfo` struct, so the buffer is 16 bytes here and only its first u32
        // is ever counted.
        let swr_num_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swrNumVertices"),
            size: abi::SIZE_DRAW_INFO as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let cube = env::CubeMesh::new(&device, &queue);
        let cube_map = crate::ktx::read_cube_map(ktx_path)?;

        let env_maps = env::EnvMaps::new(
            &device,
            &queue,
            &modules,
            &cube_map,
            &cube,
        )?;
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("env (renderer)"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // "hizBuild" set i: binding 0 = mip i, binding 1 = mip i + 1. Both are read/write storage
        // views of the same texture, so the layout is written by hand - and because it is written by
        // hand, `genhiz` cannot use `layout: None`: wgpu 30 compares bind group layouts by identity
        // (`wgpu-core-30.0.1/src/command/bind.rs:62`, `is_equal`), and an auto-derived layout is a
        // *different* object even when its entries match, which fails with
        // "Exclusive pipelines don't match: expected ComputePipeline with 'genhiz' label, got None".
        // So the layout has to exist before the pipeline and be handed to both.
        let hiz_build_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hizBuild"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let hiz_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hizBuild"),
            bind_group_layouts: &[Some(&hiz_build_bgl)],
            immediate_size: 0,
        });

        let targets = create_raster_targets(&device, width, height);

        // prepareUniformBuffers. The object UBO's `model` is `modelMats[0]`, which is what
        // `hwrasterize`/`swrasterize` bind as `uniformBuffers.object`; the per-instance matrices
        // themselves live in `modelMatsBuffer`.
        let ubo_object = uniform_buffer(&device, "ubo object", abi::SIZE_UBO_MATRICES);
        let ubo_skybox = uniform_buffer(&device, "ubo skybox", abi::SIZE_UBO_MATRICES);
        let ubo_params = uniform_buffer(&device, "ubo params", abi::SIZE_UBO_PARAMS);
        let ubo_culling = uniform_buffer(&device, "ubo culling", abi::SIZE_CULLING_MATRICES);
        let ubo_error = uniform_buffer(&device, "ubo error", abi::SIZE_ERROR_MATRICES);
        let ubo_shading = uniform_buffer(&device, "ubo shading", abi::SIZE_SHADING_UBO);

        let culling_model = Mat4::from_quat(glam::Quat::from_rotation_y(-90.0f32.to_radians()));
        {
            let mut buf = Vec::new();
            abi::encode_ubo_params(abi::default_lights(), 4.5, 2.2, &mut buf);
            queue.write_buffer(&ubo_params, 0, &buf);
            let mut buf = Vec::new();
            abi::encode_culling_matrices(
                culling_model,
                Mat4::IDENTITY,
                Mat4::IDENTITY,
                Mat4::IDENTITY,
                Mat4::IDENTITY,
                &mut buf,
            );
            queue.write_buffer(&ubo_culling, 0, &buf);
            let mut buf = Vec::new();
            abi::encode_error_matrices(
                Mat4::IDENTITY,
                Mat4::IDENTITY,
                Vec3::Y,
                Vec3::X,
                &mut buf,
            );
            queue.write_buffer(&ubo_error, 0, &buf);
            // `hwrDrawIndexedIndirectBuffer` starts zeroed in C++ and `culling.comp` only ever
            // `atomicAdd`s `indexCount`. The port draws *non-indexed*, so the same buffer's first
            // u32 doubles as `vertex_count`; `instance_count` is 1, not the C++ 2, because the Rust
            // scene has one instance slot per draw (see `createScene2`'s two `modelMats`).
            let mut buf = Vec::new();
            abi::encode_hwr_draw_indirect(0, 1, &mut buf);
            queue.write_buffer(&hwr_draw_indirect, 0, &buf);
        }

        // The HiZ taps have no sampler at all: `occlusionCulling` in `common.wgsl` uses
        // `textureLoad`, which matches the C++ `hizbuffer.sampler` (`VK_FILTER_NEAREST` +
        // `CLAMP_TO_EDGE`) exactly and keeps `R32Float`'s non-filterability out of the way.

        // ---- preparePipelines ----
        //
        // Every nanite pipeline uses `layout: None`, i.e. wgpu derives the bind group layouts *and*
        // the immediate block from the WGSL. That is the only correct option here: setting
        // `immediate_size` by hand risks a mismatch with naga's own slot count.
        let bvh_traversal = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("bvhtraversal"),
            layout: None,
            module: &modules[shaders::BVH_TRAVERSAL],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let error = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("error"),
            layout: None,
            module: &modules[shaders::ERROR],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let culling = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("culling"),
            layout: None,
            module: &modules[shaders::CULLING],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sw_rasterize = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("swrasterize"),
            layout: None,
            module: &modules[shaders::SWRASTERIZE],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let clear_image = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("clearimage"),
            layout: None,
            module: &modules[shaders::CLEAR_IMAGE],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let merger = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("merger"),
            layout: None,
            module: &modules[shaders::MERGER],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let depth_copy = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("depthcopy"),
            layout: None,
            module: &modules[shaders::DEPTH_COPY],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let gen_hiz = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("genhiz"),
            // Explicit: see the `hiz_build_bgl` comment above.
            layout: Some(&hiz_layout),
            module: &modules[shaders::GEN_HIZ],
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // hwrasterize: the base state is `depthTest/write = FALSE, LESS_OR_EQUAL, CULL_BACK,
        // FRONT_FACE_COUNTER_CLOCKWISE` and `preparePipelines` turns depth **on** for this one.
        // Two colour attachments, see the module docs.
        let hw_rasterize = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hwrasterize"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &modules[shaders::HWRASTERIZE],
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                // No vertex buffers: the draw is non-indexed and every input comes from storage
                // buffers indexed by `vertex_index`.
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &modules[shaders::HWRASTERIZE],
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::R32Uint,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::R32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            multiview_mask: None,
            cache: None,
        });

        // shading: `pipelines.shading` inherits the debug-quad state, i.e. cull NONE and depth off.
        let shading = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shading"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &modules[shaders::SHADING],
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            // The full-screen quad must not test or write the main pass' depth buffer: the shading
            // pass *is* the depth pass, it reads the visibility buffer instead. In C++ that is
            // `depthTestEnable = VK_FALSE` (`pbrtexture.cpp:2006`); in wgpu it is
            // [`depth_state_off`] - the pass still has a depth attachment, and the pipeline has to
            // name the same format.
            depth_stencil: Some(depth_state_off()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &modules[shaders::SHADING],
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // skybox: `preparePipelines` sets `cullMode = FRONT` (the cube is viewed from inside) and
        // leaves depth off. `VK_FRONT_FACE_COUNTER_CLOCKWISE` is the initializer default, which is
        // wgpu's `Ccw`; the winding only matches because [`crate::camera::flip_projection_y`]
        // cancels wgpu's Y-flipped viewport - without it, `Front` culls the whole cube away.
        let skybox = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skybox"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &modules[shaders::SKYBOX],
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(env::CubeMesh::vertex_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Front),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(depth_state_off()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &modules[shaders::SKYBOX],
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let bvh_traversal_bgl = bvh_traversal.get_bind_group_layout(0);
        let error_bgl = error.get_bind_group_layout(0);
        let culling_bgl = culling.get_bind_group_layout(0);
        let sw_rast_bgl = sw_rasterize.get_bind_group_layout(0);
        let clear_image_bgl = clear_image.get_bind_group_layout(0);
        let merger_bgl = merger.get_bind_group_layout(0);
        let hw_rast_bgl = hw_rasterize.get_bind_group_layout(0);
        let shading_bgl = shading.get_bind_group_layout(0);
        let skybox_bgl = skybox.get_bind_group_layout(0);
        let depth_copy_bgl = depth_copy.get_bind_group_layout(0);

        // "objectDraw" set 4 = the skybox set: 0 = skybox UBO, 1 = params, 2 = environment cube,
        // 3 = its sampler (`skybox.wgsl` samples with `textureSample`, unlike the HiZ taps).
        let skybox_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox"),
            layout: &skybox_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubo_skybox.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ubo_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&env_maps.source),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&env_sampler),
                },
            ],
        });

        Ok(Self {
            device,
            queue,
            features,
            bvh_traversal,
            error,
            culling,
            sw_rasterize,
            clear_image,
            merger,
            hw_rasterize,
            shading,
            skybox,
            depth_copy,
            gen_hiz,
            bvh_traversal_bgl,
            error_bgl,
            culling_bgl,
            sw_rast_bgl,
            clear_image_bgl,
            merger_bgl,
            hw_rast_bgl,
            shading_bgl,
            depth_copy_bgl,
            hiz_build_bgl,
            ubo_object,
            ubo_skybox,
            ubo_params,
            ubo_culling,
            ubo_error,
            ubo_shading,
            hwr_draw_indirect,
            swr_indirect_dispatch,
            swr_num_vertices,
            env_sampler,
            cube,
            skybox_bind_group,
            env_maps,
            targets,
            nanite: None,
            culling_model,
            object_model: Mat4::IDENTITY,
            loaded_scene: None,
            last_view: Mat4::IDENTITY,
            last_proj: Mat4::IDENTITY,
            width,
            height,
            frame_index: 0,
        })
    }

    /// The generated environment maps, for the overlay / diagnostics.
    pub fn env_maps(&self) -> &env::EnvMaps {
        &self.env_maps
    }

    /// The main pass' depth attachment, i.e. the `Depth32Float` image [`Renderer::new`] already
    /// created at the current width/height.
    ///
    /// [`Renderer::render`] takes the colour and depth views from the caller so that the same frame
    /// recording drives a swapchain and an offscreen capture; a swapchain caller has no reason to
    /// make its own depth buffer, and this is the view it should pass. It is re-created by
    /// [`Renderer::resize`], so a caller must not cache it across a resize.
    pub fn frame_depth_view(&self) -> wgpu::TextureView {
        self.targets.depth_stencil_view.clone()
    }

    /// The loaded scene's summary, or `None` before a model is loaded.
    pub fn scene_summary(&self) -> Option<String> {
        self.nanite
            .as_ref()
            .map(|(b, _)| format!("{} scene indices", b.scene_indices_count))
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// `createBVHTraversalBuffers` + `createCullingBuffers` + `createErrorProjectionBuffer` +
    /// `createModelMatsBuffer` + `writeDescriptors`, i.e. everything a model load replaces.
    ///
    /// The scene index/vertex buffers are *not* re-uploaded here: `NaniteScene::createVertexIndexBuffer`
    /// uploads them once per scene, and the caller hands the finished scene in.
    pub fn load_scene(&mut self, scene: &crate::scene::Scene) -> Result<()> {
        crate::scene::check_index_ranges(scene)?;
        let device = &self.device;
        let queue = &self.queue;

        let storage = |label: &str, bytes: &[u8], extra: wgpu::BufferUsages| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | extra,
                mapped_at_creation: false,
            });
            if !bytes.is_empty() {
                queue.write_buffer(&buffer, 0, bytes);
            }
            buffer
        };
        let u32s = |v: &[u32]| -> Vec<u8> {
            v.iter().flat_map(|x| x.to_le_bytes()).collect()
        };
        let zeros = |n: usize| vec![0u8; n];

        // ---- createBVHTraversalBuffers ----
        let mut bvh_bytes = Vec::with_capacity(scene.bvh_node_infos.len() * abi::SIZE_BVH_NODE_INFO);
        for node in &scene.bvh_node_infos {
            abi::encode_bvh_node_info(node, &mut bvh_bytes);
        }
        let bvh_node_infos = storage("bvhNodeInfos", &bvh_bytes, wgpu::BufferUsages::empty());

        let init_node_info_bytes = (scene.init_node_info_indices.len() * 4) as u64;
        let init_node_infos = storage(
            "initNodeInfos",
            &u32s(&scene.init_node_info_indices),
            // The source of the per-frame copy.
            wgpu::BufferUsages::COPY_SRC,
        );
        // `maxDepthCounts * 2` uints each, so the ping-pong can never overflow.
        let node_list_bytes = (scene.max_depth_counts.max(1) as usize * 2) * 4;
        let curr_node_infos = storage("currNodeInfos", &zeros(node_list_bytes), wgpu::BufferUsages::empty());
        let next_node_infos = storage("nextNodeInfos", &zeros(node_list_bytes), wgpu::BufferUsages::empty());
        let sorted_cluster_indices = storage(
            "sortedClusterIndices",
            &u32s(&scene.sorted_cluster_indices),
            wgpu::BufferUsages::empty(),
        );
        // "save the first 5 uint32_t for atomic counters" - `culledClusterIndices` is
        // `{clusterSize, frustumCullingNum, occulusionCullingNum, errorCullingNum, clusters[]}`.
        let culled_cluster_indices = storage(
            "culledClusterIndices",
            &zeros((scene.max_cluster_num as usize + 5) * 4),
            wgpu::BufferUsages::empty(),
        );
        let culled_cluster_object_indices = storage(
            "culledClusterObjectIndices",
            &zeros(scene.max_cluster_num as usize * 4),
            wgpu::BufferUsages::empty(),
        );

        // ---- createCullingBuffers ----
        //
        // The raster output buffers are sized to `raster_capacity()`, i.e. the *exact* upper bound
        // on what the cull shaders can emit. They used to be `sceneIndicesCount / 8` in C++, which
        // silently assumes LOD selection discards seven eighths of the scene - see AGENTS.md. The
        // hw/sw split means each buffer alone needs full capacity.
        let capacity = scene.raster_capacity() as usize;
        let hw_r_indices = storage("HWRIndices", &zeros(capacity * 4), wgpu::BufferUsages::INDEX);
        let hw_r_id = storage("HWRID", &zeros(capacity * 4), wgpu::BufferUsages::empty());
        let hw_r_vertex_ids = storage("HWRVertexIds", &zeros(capacity * 4), wgpu::BufferUsages::empty());
        let sw_r_indices = storage("SWRIndices", &zeros(capacity * 4), wgpu::BufferUsages::INDEX);
        let sw_r_id = storage("SWRID", &zeros(capacity * 4), wgpu::BufferUsages::empty());

        // `ClusterInfo` is memcpy'd, so the byte order is the C struct's: `pMinWorld` at 0, pad to
        // 16, `pMaxWorld` at 16 (ending at 28, no pad), then the three u32s at 28/32/36 and eight
        // trailing pad bytes. `nanite_blob::ClusterInfo::encode` is the same layout.
        let mut cluster_bytes =
            Vec::with_capacity(scene.cluster_infos.len() * abi::SIZE_CLUSTER_INFO);
        for c in &scene.cluster_infos {
            for v in c.p_min_world {
                cluster_bytes.extend_from_slice(&v.to_le_bytes());
            }
            cluster_bytes.extend_from_slice(&[0; 4]);
            for v in c.p_max_world {
                cluster_bytes.extend_from_slice(&v.to_le_bytes());
            }
            cluster_bytes.extend_from_slice(&c.triangle_indices_start.to_le_bytes());
            cluster_bytes.extend_from_slice(&c.triangle_indices_end.to_le_bytes());
            cluster_bytes.extend_from_slice(&c.object_idx.to_le_bytes());
            cluster_bytes.extend_from_slice(&[0; 8]);
        }
        debug_assert_eq!(cluster_bytes.len(), scene.cluster_infos.len() * abi::SIZE_CLUSTER_INFO);
        let clusters_info = storage("clustersInfo", &cluster_bytes, wgpu::BufferUsages::INDEX);

        let mut error_bytes = Vec::with_capacity(scene.error_infos.len() * 48);
        for e in &scene.error_infos {
            abi::encode_error_info(
                e.error_world,
                e.center_r.into(),
                e.center_rp.into(),
                &mut error_bytes,
            );
        }
        let error_info = storage("errorInfo", &error_bytes, wgpu::BufferUsages::INDEX);

        let projected_error = storage(
            "projectedError",
            &zeros(scene.max_cluster_num as usize * 8),
            wgpu::BufferUsages::empty(),
        );

        let mut model_mat_bytes = Vec::with_capacity(scene.instances.len() * 64);
        for instance in &scene.instances {
            for col in instance.root_transform.to_cols_array().chunks(4) {
                for c in col {
                    model_mat_bytes.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
        let model_mats = storage("modelMats", &model_mat_bytes, wgpu::BufferUsages::INDEX);

        // The scene vertex/index buffers (`NaniteScene::createVertexIndexBuffer`). The C++ uploads
        // them once per scene; a reload destroys and re-creates them, which is what happens here
        // because a new `Scene` means new buffers.
        let size_vertex = abi::SIZE_VERTEX;
        let mut vertex_bytes = Vec::with_capacity(scene.vertices.len() * size_vertex);
        for v in &scene.vertices {
            abi::encode_vertex(v, &mut vertex_bytes);
        }
        debug_assert_eq!(vertex_bytes.len(), scene.vertices.len() * size_vertex);
        let vertices = storage("scene vertices", &vertex_bytes, wgpu::BufferUsages::VERTEX);
        let indices = storage(
            "scene indices",
            &u32s(&scene.indices),
            wgpu::BufferUsages::INDEX,
        );

        // ---- writeDescriptors ----
        let b = NaniteBuffers {
            scene_vertices: vertices,
            scene_indices: indices,
            bvh_node_infos,
            init_node_infos,
            curr_node_infos,
            next_node_infos,
            sorted_cluster_indices,
            culled_cluster_indices,
            culled_cluster_object_indices,
            hw_r_indices,
            hw_r_id,
            hw_r_vertex_ids,
            sw_r_indices,
            sw_r_id,
            clusters_info,
            error_info,
            projected_error,
            model_mats,
            init_node_info_bytes,
            depth_counts: scene.depth_counts.clone(),
            max_cluster_num: scene.max_cluster_num,
            scene_indices_count: scene.scene_indices_count,
        };
        let t = &self.targets;

        let group = |label: &str,
                    layout: &wgpu::BindGroupLayout,
                    entries: &[wgpu::BindGroupEntry<'_>]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries,
            })
        };

        // "bvhTraversal" sets 0 and 1: the curr/next node lists swap.
        let bvh_traversal = [
            group(
                "bvhTraversal 0",
                &self.bvh_traversal_bgl,
                &[
                    wgpu::BindGroupEntry { binding: 0, resource: b.bvh_node_infos.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: b.curr_node_infos.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: b.next_node_infos.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: b.culled_cluster_indices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: self.ubo_culling.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.hiz_views[0]) },
                    wgpu::BindGroupEntry { binding: 6, resource: self.ubo_error.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: b.culled_cluster_object_indices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: b.sorted_cluster_indices.as_entire_binding() },
                ],
            ),
            group(
                "bvhTraversal 1",
                &self.bvh_traversal_bgl,
                &[
                    wgpu::BindGroupEntry { binding: 0, resource: b.bvh_node_infos.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: b.next_node_infos.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: b.curr_node_infos.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: b.culled_cluster_indices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: self.ubo_culling.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.hiz_views[0]) },
                    wgpu::BindGroupEntry { binding: 6, resource: self.ubo_error.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: b.culled_cluster_object_indices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 8, resource: b.sorted_cluster_indices.as_entire_binding() },
                ],
            ),
        ];

        // "errorProj"
        let error_proj = group(
            "errorProj",
            &self.error_bgl,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: b.error_info.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b.projected_error.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.ubo_error.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: b.culled_cluster_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: b.culled_cluster_object_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: b.model_mats.as_entire_binding() },
            ],
        );

        // "culling"
        let culling = group(
            "culling",
            &self.culling_bgl,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: b.clusters_info.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b.scene_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: b.hw_r_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: self.hwr_draw_indirect.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.ubo_culling.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.hiz_views[0]) },
                wgpu::BindGroupEntry { binding: 7, resource: b.projected_error.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: b.hw_r_id.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: b.sw_r_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: b.sw_r_id.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: self.swr_num_vertices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 12, resource: b.culled_cluster_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 13, resource: b.culled_cluster_object_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 14, resource: b.model_mats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 15, resource: b.hw_r_vertex_ids.as_entire_binding() },
            ],
        );

        // "swRast"
        let sw_rast = group(
            "swRast",
            &self.sw_rast_bgl,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: b.scene_vertices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b.sw_r_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: b.model_mats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: b.sw_r_id.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: self.swr_num_vertices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.sw_r_view) },
                wgpu::BindGroupEntry { binding: 6, resource: self.ubo_object.as_entire_binding() },
            ],
        );

        // "hwRast": 0 modelMats, 1 HWRVertexIds, 2 UBO, plus the scene vertex buffer and the scene
        // index buffer, which the WGSL vertex stage reads by `vertex_index` instead of through a
        // vertex input (no geometry shader). Binding 1 is the *per-vertex* packed id `culling.wgsl`
        // wrote at binding 15, not the `vec3<u32>` `outIdsHw` - see `shaders/glsl/pbrtexture/hwrasterize.geom`.
        let hw_rast = group(
            "hwRast",
            &self.hw_rast_bgl,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: b.model_mats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b.hw_r_vertex_ids.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.ubo_object.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: b.scene_vertices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: b.hw_r_indices.as_entire_binding() },
            ],
        );

        // "shading"
        let shading = group(
            "shading",
            &self.shading_bgl,
            &[
                wgpu::BindGroupEntry { binding: 0, resource: b.clusters_info.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b.scene_vertices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: b.scene_indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: b.model_mats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.final_vis_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&t.final_z_view) },
                wgpu::BindGroupEntry { binding: 6, resource: self.ubo_shading.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.ubo_params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&self.env_maps.irradiance) },
                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&self.env_maps.brdf_lut) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&self.env_maps.prefilter) },
                wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::Sampler(&self.env_sampler) },
            ],
        );

        // The scene vertex/index buffers are owned by `NaniteBuffers` (they were moved in above), so
        // they outlive the bind groups that reference them and are replaced together on the next load.
        self.object_model = scene.instances.first().map_or(Mat4::IDENTITY, |i| i.root_transform);
        self.loaded_scene = Some(scene.clone());
        self.nanite = Some((
            b,
            NaniteBindGroups {
                bvh_traversal,
                error_proj,
                culling,
                sw_rast,
                hw_rast,
                shading,
            },
        ));
        Ok(())
    }

    /// `VulkanExampleBase::resize` + `createHiZBuffer` + `setupDepthStencil` +
    /// `createHWRasterizeFramebuffer` + `createRasterizeBuffer`'s images.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }
        // Drop the nanite groups *before* the targets: every bind group references a target view,
        // and a resize invalidates all of them. The scene payload itself is kept in
        // `loaded_scene`, so the groups can be rebuilt against the new targets - which is what
        // `reloadModel()` does for the same reason.
        self.nanite = None;
        self.targets = create_raster_targets(&self.device, width, height);
        self.width = width;
        self.height = height;
        // The HiZ mip count depends on the extent, so the sampler-free HiZ taps stay correct: the
        // shaders read `textureDimensions(lastHZB, level)` rather than a baked size.
        if let Some(scene) = self.loaded_scene.clone() {
            self.load_scene(&scene)?;
        }
        Ok(())
    }

    /// Record and submit one frame, then read the two raster counters back.
    ///
    /// `color_view` / `depth_view` are the main render pass' attachments: the swapchain image and
    /// its depth buffer when presenting, or an offscreen `COPY_SRC` target when capturing. The
    /// renderer's own [`RasterTargets::depth_stencil_view`] is *not* used for the main pass - the
    /// caller's is, so that a capture target gets a matching depth buffer.
    pub fn render(
        &mut self,
        camera: &Camera,
        settings: &FrameSettings,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        read_raster_stats: bool,
    ) -> Result<RasterStats> {
        // updateUniformBuffers: the six model/parameter blocks. This is done *before* the frame
        // borrows `self` immutably for the buffers/bind groups below - wgpu's pass objects hold
        // references into `self` for the whole recording, so no `&mut self` call may be interleaved.
        self.write_uniforms(camera, settings);

        // draw(): `lastView/lastProj` are updated *after* the frame is submitted, i.e. the next
        // frame's occlusion test uses this frame's frustum. Nothing here reads them again, so
        // writing them now is equivalent to writing them after `submit`.
        self.last_view = camera.view;
        self.last_proj = camera.projection;
        let frame_index = self.frame_index;
        self.frame_index += 1;
        let scene_indices = self
            .nanite
            .as_ref()
            .map(|(b, _)| b.scene_indices_count)
            .unwrap_or(0);

        let (buffers, groups) = match &self.nanite {
            Some(v) => v,
            None => bail!("render called with no nanite scene loaded"),
        };
        let t = &self.targets;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });

        // buildCommandBuffers head: reset the traversal frontier.
        encoder.copy_buffer_to_buffer(
            &buffers.init_node_infos,
            0,
            &buffers.curr_node_infos,
            0,
            buffers.init_node_info_bytes,
        );
        // vkCmdFillBuffer(culledClusterIndicesBuffer, 0, 5 * sizeof(uint32_t), 0)
        encoder.clear_buffer(&buffers.culled_cluster_indices, 0, Some(20));
        // draw(): the two host-side resets, as GPU clears.
        encoder.clear_buffer(&self.hwr_draw_indirect, 0, Some(4));
        encoder.clear_buffer(&self.swr_num_vertices, 0, Some(4));

        // ---- bvhtraversal, one dispatch per depth level ----
        for (j, &count) in buffers.depth_counts.iter().enumerate() {
            if count == 0 {
                break;
            }
            let dst = if j & 1 == 1 {
                &buffers.curr_node_infos
            } else {
                &buffers.next_node_infos
            };
            encoder.clear_buffer(dst, 0, Some(4));
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bvhtraversal"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bvh_traversal);
            // wgpu resolves `set_immediates` against the *current* pipeline layout
            // (`wgpu-core-30/src/command/pass.rs`: `binder.pipeline_layout.as_ref().ok_or(
            // MissingPipeline)?`), so unlike a Vulkan `vkCmdPushConstants` it must come *after*
            // `set_pipeline`.
            pass.set_immediates(0, &bvh_traversal_immediates(self.width, self.height, settings.threshold));
            pass.set_bind_group(0, &groups.bvh_traversal[j & 1], &[]);
            pass.dispatch_workgroups(count.div_ceil(WORKGROUP_SIZE_1D), 1, 1);
        }

        // ---- error projection ----
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("error"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.error);
            pass.set_immediates(
                0,
                &error_immediates(buffers.max_cluster_num as i32, self.width, self.height),
            );
            pass.set_bind_group(0, &groups.error_proj, &[]);
            pass.dispatch_workgroups(buffers.max_cluster_num.div_ceil(WORKGROUP_SIZE_1D), 1, 1);
        }

        // ---- culling ----
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("culling"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.culling);
            pass.set_immediates(
                0,
                &culling_immediates(
                    buffers.max_cluster_num as i32,
                    settings.threshold,
                    settings.frustum_occlusion,
                    settings.software_raster,
                ),
            );
            pass.set_bind_group(0, &groups.culling, &[]);
            pass.dispatch_workgroups(buffers.max_cluster_num.div_ceil(WORKGROUP_SIZE_1D), 1, 1);
        }

        // ---- clearimage (also computes the swr indirect dispatch) ----
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("clearimage"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.clear_image);
            pass.set_bind_group(
                0,
                &self.clear_image_bind_group(),
                &[],
            );
            pass.dispatch_workgroups(
                self.width.div_ceil(WORKGROUP_SIZE_2D),
                self.height.div_ceil(WORKGROUP_SIZE_2D),
                1,
            );
        }

        // ---- swrasterize, indirect ----
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("swrasterize"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.sw_rasterize);
            pass.set_bind_group(0, &groups.sw_rast, &[]);
            pass.dispatch_workgroups_indirect(&self.swr_indirect_dispatch, 0);
        }

        // ---- hwrasterize render pass ----
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hwrasterize"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &t.hw_r_vis_view,
                        depth_slice: None,
                        // hwrasterize.frag clears to 0xFFFFFFFF per channel; the "no hit" sentinel
                        // the merger tests for.
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: u32::MAX as f64,
                                g: u32::MAX as f64,
                                b: u32::MAX as f64,
                                a: u32::MAX as f64,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &t.hwr_depth_copy_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 1.0,
                                g: 0.0,
                                b: 0.0,
                                a: 0.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth_stencil_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.hw_rasterize);
            pass.set_bind_group(0, &groups.hw_rast, &[]);
            pass.set_viewport(0.0, 0.0, self.width as f32, self.height as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, self.width, self.height);
            // Non-indexed indirect: `DrawIndirectArgs{vertex_count, instance_count, first_vertex,
            // first_instance}`. `instance_count` is 1, not C++'s 2 - the geometry shader that
            // emitted two vertices per primitive is gone, `culling.wgsl` already emitted three.
            pass.draw_indirect(&self.hwr_draw_indirect, 0);
        }

        // ---- merger ----
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("merger"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.merger);
            pass.set_immediates(0, &visualize_immediates(settings.visualize));
            pass.set_bind_group(0, &self.merge_bind_group(), &[]);
            pass.dispatch_workgroups(
                self.width.div_ceil(WORKGROUP_SIZE_2D),
                self.height.div_ceil(WORKGROUP_SIZE_2D),
                1,
            );
        }

        // ---- main render pass ----
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // buildCommandBuffers: `clearValues[0].color = {0.1, 0.1, 0.1, 1}`.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.1,
                            b: 0.1,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if settings.skybox {
                pass.set_pipeline(&self.skybox);
                pass.set_bind_group(0, &self.skybox_bind_group, &[]);
                pass.set_vertex_buffer(0, self.cube.vertex_buffer.slice(..));
                pass.set_index_buffer(self.cube.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.cube.index_count, 0, 0..1);
            }
            pass.set_pipeline(&self.shading);
            pass.set_immediates(0, &visualize_immediates(settings.visualize));
            pass.set_bind_group(0, &groups.shading, &[]);
            // vkCmdDraw(3, 1, 0, 0): the full-screen triangle.
            pass.draw(0..3, 0..1);
        }

        // ---- depthcopy: final depth -> HiZ mip 0 ----
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("depthcopy"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.depth_copy);
            pass.set_bind_group(0, &self.depth_copy_bind_group(), &[]);
            pass.dispatch_workgroups(
                self.width.div_ceil(WORKGROUP_SIZE_2D),
                self.height.div_ceil(WORKGROUP_SIZE_2D),
                1,
            );
        }

        // ---- genhiz, one dispatch per mip. The C++ dispatches the FULL-resolution grid for every
        // mip and lets the shader's output-size guard drop the extra invocations. ----
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("genhiz"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.gen_hiz);
            for mip in 0..t.hiz_mip_levels.saturating_sub(1) {
                pass.set_bind_group(0, &self.hiz_bind_groups()[mip as usize], &[]);
                pass.dispatch_workgroups(
                    self.width.div_ceil(WORKGROUP_SIZE_2D),
                    self.height.div_ceil(WORKGROUP_SIZE_2D),
                    1,
                );
            }
        }

        // The counters `--raster-stats` reads. Copied into a mappable buffer in the same submission,
        // so one poll after submit is enough.
        let readback = if read_raster_stats {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("raster stats readback"),
                size: 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&self.hwr_draw_indirect, 0, &buffer, 0, 4);
            encoder.copy_buffer_to_buffer(&self.swr_num_vertices, 0, &buffer, 4, 4);
            Some(buffer)
        } else {
            None
        };

        self.queue.submit([encoder.finish()]);

        let stats = if let Some(buffer) = readback {
            buffer.slice(0..8).map_async(wgpu::MapMode::Read, |result| {
                if let Err(e) = result {
                    tracing::warn!("raster stats readback failed: {e}");
                }
            });
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            let data = buffer.slice(0..8).get_mapped_range()?;
            let hw = u32::from_le_bytes(data[0..4].try_into().unwrap());
            let sw = u32::from_le_bytes(data[4..8].try_into().unwrap());
            RasterStats {
                scene_indices,
                hw_indices: hw,
                sw_indices: sw,
            }
        } else {
            RasterStats {
                scene_indices,
                ..Default::default()
            }
        };
        if read_raster_stats {
            // `printRasterStatsHeadless`, called from the capture branch of `render()`.
            tracing::info!("{}", Self::raster_stats_line(frame_index, &stats));
        }
        Ok(stats)
    }

    /// The `[raster] frame N ...` line of `printRasterStatsHeadless`, byte for byte. `frame_index`
    /// is `framesRendered` *before* it is incremented, exactly as the C++ prints it - so the first
    /// captured frame logs `frame 0`.
    pub fn raster_stats_line(frame_index: u64, stats: &RasterStats) -> String {
        format!(
            "[raster] frame {} scene indices {} raster slots per path {} hw tris {} sw tris {} total tris {}",
            frame_index,
            stats.scene_indices,
            stats.scene_indices,
            stats.hw_triangles(),
            stats.sw_triangles(),
            stats.total_triangles(),
        )
    }

    /// `updateUniformBuffers` + `updateParams`.
    fn write_uniforms(&mut self, camera: &Camera, settings: &FrameSettings) {
        let view = camera.view;
        let proj = camera.projection;
        let cam_pos = camera.cam_pos();

        let mut buf = Vec::new();
        // `uniformBuffers.object`: `model = modelMats[0]`, which is what `hwrasterize` and
        // `swrasterize` use as their `model`. The per-instance matrices are in `modelMatsBuffer`.
        abi::encode_ubo_matrices(proj, self.object_model, view, cam_pos, &mut buf);
        self.queue.write_buffer(&self.ubo_object, 0, &buf);

        buf.clear();
        // `uniformBuffers.skybox`: `model = mat4(mat3(view))`, i.e. the translation stripped so the
        // cube stays centred on the eye.
        abi::encode_ubo_matrices(proj, skybox_model(view), view, cam_pos, &mut buf);
        self.queue.write_buffer(&self.ubo_skybox, 0, &buf);

        buf.clear();
        abi::encode_ubo_params(abi::default_lights(), settings.exposure, settings.gamma, &mut buf);
        self.queue.write_buffer(&self.ubo_params, 0, &buf);

        buf.clear();
        abi::encode_culling_matrices(
            self.culling_model,
            self.last_view,
            self.last_proj,
            view,
            proj,
            &mut buf,
        );
        self.queue.write_buffer(&self.ubo_culling, 0, &buf);

        buf.clear();
        abi::encode_error_matrices(
            view,
            proj,
            Camera::get_up(),
            camera.get_right(),
            &mut buf,
        );
        self.queue.write_buffer(&self.ubo_error, 0, &buf);

        buf.clear();
        abi::encode_shading_ubo(view.inverse(), proj.inverse(), cam_pos, &mut buf);
        self.queue.write_buffer(&self.ubo_shading, 0, &buf);
    }

    /// The `model` slot of `uniformBuffers.object`. `load_scene` sets this to `modelMats[0]`.
    fn clear_image_bind_group(&self) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("clearImage"),
            layout: &self.clear_image_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.targets.sw_r_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.swr_num_vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.swr_indirect_dispatch.as_entire_binding(),
                },
            ],
        })
    }

    fn merge_bind_group(&self) -> wgpu::BindGroup {
        let t = &self.targets;
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mergeRast"),
            layout: &self.merger_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&t.hwr_depth_copy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&t.hw_r_vis_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&t.sw_r_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&t.final_z_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&t.final_vis_view),
                },
            ],
        })
    }

    fn depth_copy_bind_group(&self) -> wgpu::BindGroup {
        let t = &self.targets;
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("depthCopy"),
            layout: &self.depth_copy_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&t.final_z_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&t.hiz_views[0]),
                },
            ],
        })
    }

    fn hiz_bind_groups(&self) -> Vec<wgpu::BindGroup> {
        let t = &self.targets;
        (0..t.hiz_mip_levels.saturating_sub(1))
            .map(|mip| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hizBuild"),
                    layout: &self.hiz_build_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&t.hiz_views[mip as usize]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                &t.hiz_views[mip as usize + 1],
                            ),
                        },
                    ],
                })
            })
            .collect()
    }
}

/// `setupRenderPass`' hw-rast framebuffer + `createRasterizeBuffer`'s images + `createHiZBuffer` +
/// `setupDepthStencil`, as one screen-sized bundle.
fn create_raster_targets(device: &wgpu::Device, width: u32, height: u32) -> RasterTargets {
    let extent = |w: u32, h: u32| wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let texture = |label: &str,
                   size: wgpu::Extent3d,
                   format: wgpu::TextureFormat,
                   usage: wgpu::TextureUsages,
                   mip_level_count: u32| {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        t
    };
    let view = |t: &wgpu::Texture, desc: wgpu::TextureViewDescriptor| t.create_view(&desc);

    let binding = wgpu::TextureUsages::TEXTURE_BINDING;
    let storage = wgpu::TextureUsages::STORAGE_BINDING;

    // HWRVisBuffer: R32ui colour attachment of the hw-rast pass, read as a storage texture by the
    // merger.
    let hw_r_vis = texture(
        "HWRVis",
        extent(width, height),
        wgpu::TextureFormat::R32Uint,
        wgpu::TextureUsages::RENDER_ATTACHMENT | storage | binding,
        1,
    );
    // The R32Float stand-in for HWRZBuffer. `R32Float` is guaranteed `attachment | storage |
    // binding`, so one texture serves the colour attachment and the merger's read.
    let hwr_depth_copy = texture(
        "HWRZ copy",
        extent(width, height),
        wgpu::TextureFormat::R32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | storage | binding,
        1,
    );
    // SWRBuffer: R64ui, storage only. `R64Uint` has no RENDER_ATTACHMENT at all. `STORAGE_ATOMIC`
    // is required because `swrasterize.wgsl` binds it as `texture_storage_2d<r64uint, atomic>`
    // (`resource.rs` maps `Access::Atomic` to that usage); `clearimage`/`merger` use write/read.
    let sw_r = texture(
        "SWR",
        extent(width, height),
        wgpu::TextureFormat::R64Uint,
        storage | binding | wgpu::TextureUsages::STORAGE_ATOMIC,
        1,
    );
    let final_vis = texture(
        "FinalVis",
        extent(width, height),
        wgpu::TextureFormat::R32Uint,
        storage | binding,
        1,
    );
    // FinalZBuffer is a *colour* image (R32float, storage only) - the main pass' depth attachment is
    // a separate D32 image, and the shading pass reads this one.
    let final_z = texture(
        "FinalZ",
        extent(width, height),
        wgpu::TextureFormat::R32Float,
        storage | binding,
        1,
    );
    let hiz_mip_levels = env::hiz_mip_levels(width, height);
    let hiz = texture(
        "HiZ",
        extent(width, height),
        wgpu::TextureFormat::R32Float,
        storage | binding,
        hiz_mip_levels,
    );
    let depth_stencil = texture(
        "depth stencil",
        extent(width, height),
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
        1,
    );

    let hiz_views = (0..hiz_mip_levels)
        .map(|mip| {
            let label = format!("HiZ mip {mip}");
            view(
                &hiz,
                wgpu::TextureViewDescriptor {
                    label: Some(&label),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                },
            )
        })
        .collect();

    let depth_stencil_view = view(&depth_stencil, wgpu::TextureViewDescriptor::default());

    RasterTargets {
        hw_r_vis_view: view(&hw_r_vis, wgpu::TextureViewDescriptor::default()),
        hwr_depth_copy_view: view(&hwr_depth_copy, wgpu::TextureViewDescriptor::default()),
        sw_r_view: view(&sw_r, wgpu::TextureViewDescriptor::default()),
        final_vis_view: view(&final_vis, wgpu::TextureViewDescriptor::default()),
        final_z_view: view(&final_z, wgpu::TextureViewDescriptor::default()),
        hiz_views,
        hw_r_vis,
        hwr_depth_copy,
        sw_r,
        final_vis,
        final_z,
        hiz,
        hiz_mip_levels,
        depth_stencil,
        depth_stencil_view,
        width,
        height,
    }
}

/// A `wgpu::RenderPipelineDescriptor` has no `module` field - the vertex and fragment stages each
/// name their own. See the `create_render_pipeline` call sites.

fn uniform_buffer(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// `mat4(mat3(view))`: the cube stays centred on the eye.
fn skybox_model(view: Mat4) -> Mat4 {
    Mat4::from_mat3(glam::Mat3::from_mat4(view))
}

// ---------------------------------------------------------------------------
// immediate blocks (the C++ push constants)
// ---------------------------------------------------------------------------

/// `bvhtraversal.comp`'s `PushConstants`: `vec2 screenSize; float threshold;` - 12 bytes.
pub fn bvh_traversal_immediates(width: u32, height: u32, threshold: f32) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&(width as f32).to_le_bytes());
    out[4..8].copy_from_slice(&(height as f32).to_le_bytes());
    out[8..12].copy_from_slice(&threshold.to_le_bytes());
    out
}

/// `error.comp`'s `PushConstants`: `int numClusters; vec2 screenSize;` - 16 bytes, with the
/// 4-byte pad WGSL inserts after the `int`.
pub fn error_immediates(num_clusters: i32, width: u32, height: u32) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&num_clusters.to_le_bytes());
    // `vec2<f32>` is 8-byte aligned, so slot 1 is skipped.
    out[4..8].fill(0);
    out[8..12].copy_from_slice(&(width as f32).to_le_bytes());
    out[12..16].copy_from_slice(&(height as f32).to_le_bytes());
    out
}

/// `culling.comp`'s `PushConstants`: `int numClusters; float threshold; int useFrustrumOcclusion;
/// int useSoftwareRast;` - 16 bytes.
pub fn culling_immediates(
    num_clusters: i32,
    threshold: f32,
    frustum_occlusion: bool,
    software_raster: bool,
) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&num_clusters.to_le_bytes());
    out[4..8].copy_from_slice(&threshold.to_le_bytes());
    out[8..12].copy_from_slice(&(frustum_occlusion as i32).to_le_bytes());
    out[12..16].copy_from_slice(&(software_raster as i32).to_le_bytes());
    out
}

/// `merger.comp` / `shading.frag`'s `PushConstants`: `int vis_clusters;` - 4 bytes.
pub fn visualize_immediates(vis_clusters: i32) -> [u8; 4] {
    vis_clusters.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_blocks_have_the_c_sizes() {
        // The C++ push constant ranges: 12, 16, 16, 4, 4 bytes. A WGSL `var<immediate>` block is
        // laid out in 4-byte slots, so these must match exactly or `set_immediates` is rejected.
        assert_eq!(bvh_traversal_immediates(1280, 720, 0.0005).len(), 12);
        assert_eq!(error_immediates(15450, 1280, 720).len(), 16);
        assert_eq!(culling_immediates(15450, 0.0005, true, true).len(), 16);
        assert_eq!(visualize_immediates(3).len(), 4);
    }

    #[test]
    fn immediate_blocks_match_the_c_offsets() {
        let b = bvh_traversal_immediates(1280, 720, 0.5);
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 1280.0);
        assert_eq!(f32::from_le_bytes(b[4..8].try_into().unwrap()), 720.0);
        assert_eq!(f32::from_le_bytes(b[8..12].try_into().unwrap()), 0.5);

        let e = error_immediates(-7, 1280, 720);
        assert_eq!(i32::from_le_bytes(e[0..4].try_into().unwrap()), -7);
        assert_eq!(f32::from_le_bytes(e[8..12].try_into().unwrap()), 1280.0);
        assert_eq!(f32::from_le_bytes(e[12..16].try_into().unwrap()), 720.0);

        let c = culling_immediates(15450, 0.0005, true, false);
        assert_eq!(i32::from_le_bytes(c[0..4].try_into().unwrap()), 15450);
        assert_eq!(f32::from_le_bytes(c[4..8].try_into().unwrap()), 0.0005);
        assert_eq!(i32::from_le_bytes(c[8..12].try_into().unwrap()), 1);
        assert_eq!(i32::from_le_bytes(c[12..16].try_into().unwrap()), 0);
    }

    #[test]
    fn default_settings_match_the_viewer() {
        let s = FrameSettings::default();
        assert_eq!(s.threshold, 500.0 * 1e-6);
        assert!(s.frustum_occlusion);
        assert!(s.software_raster);
        assert!(s.skybox);
        assert_eq!(s.visualize, 0);
        assert_eq!(s.exposure, 4.5);
        assert_eq!(s.gamma, 2.2);
    }

    #[test]
    #[allow(non_snake_case)]
    fn raster_stats_line_matches_printRasterStatsHeadless() {
        let stats = RasterStats {
            scene_indices: 288_092,
            hw_indices: 286_664,
            sw_indices: 410,
        };
        assert_eq!(stats.hw_triangles(), 95_554);
        assert_eq!(stats.sw_triangles(), 136);
        assert_eq!(stats.total_triangles(), 95_691);
        let line = Renderer::raster_stats_line(0, &stats);
        assert_eq!(
            line,
            "[raster] frame 0 scene indices 288092 raster slots per path 288092 hw tris 95554 sw tris 136 total tris 95691"
        );
    }

    #[test]
    fn required_features_are_the_nanite_set() {
        let f = REQUIRED_FEATURES;
        for needed in [
            wgpu::Features::IMMEDIATES,
            wgpu::Features::SHADER_INT64,
            wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX,
            wgpu::Features::TEXTURE_INT64_ATOMIC,
            wgpu::Features::TEXTURE_ATOMIC,
        ] {
            assert!(f.contains(needed), "{needed:?} missing from REQUIRED_FEATURES");
        }
    }

    #[test]
    fn raster_formats_are_bindable_and_attachable() {
        // The whole hw-rast/merger design rests on these guarantees. If a future wgpu changes the
        // table, this fails here rather than as a validation error at draw time.
        let f = wgpu::Features::TEXTURE_INT64_ATOMIC | wgpu::Features::TEXTURE_ATOMIC;
        let r32f = wgpu::TextureFormat::R32Float.guaranteed_format_features(f);
        assert!(r32f.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(r32f.allowed_usages.contains(wgpu::TextureUsages::STORAGE_BINDING));
        assert!(r32f.allowed_usages.contains(wgpu::TextureUsages::TEXTURE_BINDING));

        let r32ui = wgpu::TextureFormat::R32Uint.guaranteed_format_features(f);
        assert!(r32ui.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(r32ui.allowed_usages.contains(wgpu::TextureUsages::STORAGE_BINDING));

        let r64ui = wgpu::TextureFormat::R64Uint.guaranteed_format_features(f);
        assert!(r64ui.allowed_usages.contains(wgpu::TextureUsages::STORAGE_BINDING));
        assert!(r64ui.allowed_usages.contains(wgpu::TextureUsages::STORAGE_ATOMIC));
        // ... and it must NOT be usable as a colour attachment: the C++ SWRBuffer is storage only.
        assert!(!r64ui.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
    }

    #[test]
    fn skybox_model_strips_the_translation() {
        let view = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let m = skybox_model(view);
        assert_eq!(m.w_axis.truncate(), Vec3::ZERO);
        assert_eq!(m.transform_point3(Vec3::X), Vec3::X);
    }
}
