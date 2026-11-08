//! Environment-map generation and the HZB mip-count helper.
//!
//! Three one-time passes run after the KTX cube map has been uploaded, mirroring
//! `generateIrradianceCube` (`examples/pbrtexture/pbrtexture.cpp:2376`),
//! `generatePrefilteredCube` (`:2756`) and `generateBRDFLUT` (`:2320`), plus the mip-count rule of
//! `createHiZBuffer` (`:3761`).
//!
//! Three structural differences from the C++ version are worth knowing before reading the code:
//!
//! - The C++ example renders each cube face into an offscreen 2D image and then `vkCmdCopyImage`s
//!   it into layer *f*, mip *m* of the cube image. wgpu lets a render pass attach a `D2` view of a
//!   2D-array texture directly (`wgpu-core/src/device/resource.rs:1951` only requires exactly one
//!   array layer for a `D2` view), so each pass targets `(mip m, face f)` of the cube itself.
//! - `Rg16Float` (the BRDF LUT) has no `STORAGE_BINDING` in wgpu's guaranteed format-feature table
//!   (`wgpu-types/src/texture/format.rs`), so the LUT has to come from a render pass - which is
//!   what the C++ example does anyway.
//! - `R32Float`/`Rgba32Float` are only filterable when `Features::FLOAT32_FILTERABLE` is
//!   requested - the wgpu Vulkan backend advertises it when the driver supports it, but nothing here
//!   asks for it. `shading.wgsl` samples the *irradiance* cube with `textureSample`, so that cube is
//!   `Rgba16Float` here instead of C++'s `Rgba32Float`: `Rgba16Float` is a filterable float sample
//!   type unconditionally. The source cube is already `Rgba16Float` (that is what the KTX holds), so
//!   the two generator passes keep sampling it with hardware interpolation exactly as the C++
//!   generators do.
//!
//! The immediate blocks (`var<immediate>`) replace the C++ push constant ranges; their layouts are
//! pinned by [`irradiance_immediates`] and [`prefilter_immediates`] against `struct PushBlock` at
//! `pbrtexture.cpp:2567` and `:2947`.

use anyhow::{Result, bail};
use glam::{Mat4, Quat, Vec3};

use crate::ktx::{self, CubeMap};
use crate::shaders;

/// Cube-map face order, matching `VkImageSubresourceRange` layer order and the face view matrices
/// in `pbrtexture.cpp:2600-2640`.
pub const FACES: [&str; 6] = [
    "positive_x",
    "negative_x",
    "positive_y",
    "negative_y",
    "positive_z",
    "negative_z",
];

/// Irradiance cube edge length (`pbrtexture.cpp:2381`).
pub const IRRADIANCE_DIM: u32 = 64;
/// Prefiltered cube edge length (`pbrtexture.cpp:2761`).
pub const PREFILTER_DIM: u32 = 512;
/// BRDF LUT edge length.
pub const BRDF_DIM: u32 = 512;

/// `floor(log2(dim)) + 1`, the mip count the C++ example derives with `log2` (`:2382`).
pub const fn mip_count(dim: u32) -> u32 {
    32 - dim.leading_zeros()
}

/// `mipLevels = floor(log2(max(width, height))) + 1` for the HZB (`pbrtexture.cpp:3762`).
pub fn hiz_mip_levels(width: u32, height: u32) -> u32 {
    mip_count(width.max(height))
}

/// The six view matrices used to render one cube face each.
///
/// Verbatim from `pbrtexture.cpp:2628-2640`. `glm::rotate` **post-multiplies**: glm builds
/// `Result = m * R(angle, axis)`, so `glm::rotate(glm::rotate(I, a, A), b, B)` is `R(a,A) * R(b,B)`
/// in glam - the *inner* call is the **left** factor.
pub fn face_view_matrices() -> [Mat4; 6] {
    let (x, y, z) = (Vec3::X, Vec3::Y, Vec3::Z);
    let rot = |angle: f32, axis: Vec3| Mat4::from_axis_angle(axis, angle);
    [
        // POSITIVE_X: rotate(rotate(I, 90, Y), 180, X) == Ry(90) * Rx(180)
        rot(std::f32::consts::FRAC_PI_2, y) * rot(std::f32::consts::PI, x),
        // NEGATIVE_X: rotate(rotate(I, -90, Y), 180, X) == Ry(-90) * Rx(180)
        rot(-std::f32::consts::FRAC_PI_2, y) * rot(std::f32::consts::PI, x),
        // POSITIVE_Y: rotate(I, -90, X)
        rot(-std::f32::consts::FRAC_PI_2, x),
        // NEGATIVE_Y: rotate(I, 90, X)
        rot(std::f32::consts::FRAC_PI_2, x),
        // POSITIVE_Z: rotate(I, 180, X)
        rot(std::f32::consts::PI, x),
        // NEGATIVE_Z: rotate(I, 180, Z)
        rot(std::f32::consts::PI, z),
    ]
}

/// `perspective(PI/2, 1.0, 0.1, 512)` - the projection both cube generators push (`:2650`).
/// `glm::perspective` equals [`Mat4::perspective_rh`] for glam 0.29 (zero-to-one depth range).
///
/// It then goes through [`camera::flip_projection_y`]: the C++ pass draws with a positive-height
/// viewport (`pbrtexture.cpp:2644`), so its face row 0 is NDC y = -1, while wgpu's flipped viewport
/// puts row 0 at NDC y = +1. Without the flip every generated face - and therefore the whole IBL -
/// comes out a vertical mirror of the C++ one, even though the *sampling* side (direction-based
/// cube taps) is identical in both.
pub fn cube_projection() -> Mat4 {
    crate::camera::flip_projection_y(Mat4::perspective_rh(
        std::f32::consts::FRAC_PI_2,
        1.0,
        0.1,
        512.0,
    ))
}

/// Both cube generators push `mat4 mvp` followed by two 4-byte scalars, so both immediate blocks
/// are the same size: 64 + 4 + 4 = 72. The C++ `struct PushBlock` reports `sizeof == 80` because
/// `glm::mat4` forces 16-byte struct alignment, but the C++ push range only covers the used bytes,
/// and naga's `var<immediate>` only requires the slots the shader actually reads (mvp = 0..15,
/// the two scalars = 16 and 17).
pub const CUBE_PUSH_SIZE: usize = 72;

/// `PushBlock` of `generateIrradianceCube` (`pbrtexture.cpp:2567-2572`):
/// `mat4 mvp; float deltaPhi = 2*PI/180; float deltaTheta = 0.5*PI/64;`
pub const IRRADIANCE_IMMEDIATE_SIZE: u32 = CUBE_PUSH_SIZE as u32;

/// Encode [`CUBE_PUSH_SIZE`] bytes of immediate data for `irradiancecube.wgsl`.
pub fn irradiance_immediates(mvp: Mat4) -> [u8; CUBE_PUSH_SIZE] {
    let mut out = [0u8; CUBE_PUSH_SIZE];
    write_mat4(&mut out, 0, mvp);
    write_f32(&mut out, 64, std::f32::consts::TAU / 180.0);
    write_f32(&mut out, 68, std::f32::consts::FRAC_PI_2 / IRRADIANCE_DIM as f32);
    out
}

/// `PushBlock` of `generatePrefilteredCube` (`pbrtexture.cpp:2947-2951`):
/// `mat4 mvp; float roughness; uint32_t numSamples = 32;`
pub const PREFILTER_IMMEDIATE_SIZE: u32 = CUBE_PUSH_SIZE as u32;
/// `numSamples` default from the same struct.
pub const PREFILTER_NUM_SAMPLES: u32 = 32;

/// Encode [`CUBE_PUSH_SIZE`] bytes of immediate data for `prefilterenvmap.wgsl`.
pub fn prefilter_immediates(mvp: Mat4, roughness: f32) -> [u8; CUBE_PUSH_SIZE] {
    let mut out = [0u8; CUBE_PUSH_SIZE];
    write_mat4(&mut out, 0, mvp);
    write_f32(&mut out, 64, roughness);
    write_u32(&mut out, 68, PREFILTER_NUM_SAMPLES);
    out
}

/// `roughness = mip / (numMips - 1)` (`pbrtexture.cpp:2985`), the value the C++ loop pushes once
/// per mip. `MAX_REFLECTION_LOD` in `common.wgsl` is that same `numMips - 1`.
pub fn prefilter_roughness(mip: u32) -> f32 {
    mip as f32 / (mip_count(PREFILTER_DIM) - 1) as f32
}

fn write_f32(buf: &mut [u8], at: usize, value: f32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_mat4(buf: &mut [u8], at: usize, m: Mat4) {
    // glam stores column-major, which is what both std140 and WGSL expect.
    for (c, col) in m.to_cols_array().chunks(4).enumerate() {
        for (r, v) in col.iter().enumerate() {
            write_f32(buf, at + c * 16 + r * 4, *v);
        }
    }
}

/// The 24 positions of `assets/models/cube.gltf`, accessor 0, in file order.
pub const CUBE_POSITIONS: [[f32; 3]; 24] = [
    [-5.0, -5.0, 5.0],
    [-5.0, -5.0, -5.0],
    [5.0, -5.0, -5.0],
    [5.0, -5.0, 5.0],
    [-5.0, 5.0, 5.0],
    [5.0, 5.0, 5.0],
    [5.0, 5.0, -5.0],
    [-5.0, 5.0, -5.0],
    [-5.0, -5.0, 5.0],
    [5.0, -5.0, 5.0],
    [5.0, 5.0, 5.0],
    [-5.0, 5.0, 5.0],
    [5.0, -5.0, 5.0],
    [5.0, -5.0, -5.0],
    [5.0, 5.0, -5.0],
    [5.0, 5.0, 5.0],
    [5.0, -5.0, -5.0],
    [-5.0, -5.0, -5.0],
    [-5.0, 5.0, -5.0],
    [5.0, 5.0, -5.0],
    [-5.0, -5.0, -5.0],
    [-5.0, -5.0, 5.0],
    [-5.0, 5.0, 5.0],
    [-5.0, 5.0, -5.0],
];

/// The 36 indices of `assets/models/cube.gltf`, accessor 3 (`u16` in the file, `u32` on the GPU -
/// `vkglTF::Model::draw` binds `VK_INDEX_TYPE_UINT32`).
pub const CUBE_INDICES: [u32; 36] = [
    0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17, 18,
    16, 18, 19, 20, 21, 22, 20, 22, 23,
];

/// The node matrix of the `cube` node in `cube.gltf`: a quaternion rotation, no translation.
///
/// `vkglTF::Model::draw` never pushes a model matrix per node - it binds one `uboMatrices` and
/// draws every node against it (`base/VulkanglTFModel.cpp:1448-1476`) - so the only node transform
/// that reaches the GPU is the one baked into the vertex buffer by
/// `FileLoadingFlags::PreTransformVertices` (`base/VulkanglTFModel.cpp:1275`), which
/// `NaniteMesh::kBuilderLoadingFlags` always requests. `FlipY` then negates `pos.y` (`:1280`).
/// Both are applied here so the Rust viewer's cube matches the bytes the C++ example uploads.
pub fn cube_node_matrix() -> Mat4 {
    Mat4::from_quat(Quat::from_xyzw(
        0.7071068286895752,
        0.0,
        0.0,
        0.7071067094802856,
    ))
}

/// [`CUBE_POSITIONS`] after `PreTransformVertices` + `FlipY`, i.e. what the C++ vertex buffer holds.
pub fn cube_positions_transformed() -> [[f32; 3]; 24] {
    let m = cube_node_matrix();
    CUBE_POSITIONS.map(|p| {
        let q = m.transform_point3(Vec3::from_array(p));
        [q.x, -q.y, q.z]
    })
}

/// The skybox cube: 24 vertices spanning +/-5 and 36 indices, which is what
/// `models.skybox.draw()` / `models.cube.draw()` submit in C++.
///
/// Positions only - `skybox.vert`, `filtercube.vert` and `genbrdflut.vert` never read a normal or
/// a uv, and the WGSL ports declare only `@location(0) inPos: vec3<f32>`.
pub struct CubeMesh {
    /// `vec3` positions, one per vertex, `Float32x3` with stride 12.
    pub vertex_buffer: wgpu::Buffer,
    /// `u32` index buffer.
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
}

impl CubeMesh {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let vertex_data: Vec<u8> = cube_positions_transformed()
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
            .collect();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skybox cube vertices"),
            size: vertex_data.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, &vertex_data);

        let index_data: Vec<u8> = CUBE_INDICES.iter().flat_map(|i| i.to_le_bytes()).collect();
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("skybox cube indices"),
            size: index_data.len() as u64,
            usage: wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, &index_data);

        Self {
            vertex_buffer,
            index_buffer,
            index_count: CUBE_INDICES.len() as u32,
        }
    }

    /// The `Float32x3` vertex layout both cube pipelines use.
    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            }],
        }
    }
}

/// Byte size of one texel of a format [`ktx`] can produce. Kept next to the upload because the row
/// padding depends on it.
fn bytes_per_texel(format: wgpu::TextureFormat) -> Result<usize> {
    Ok(match format {
        wgpu::TextureFormat::Rgba8Unorm
        | wgpu::TextureFormat::Rgba8UnormSrgb
        | wgpu::TextureFormat::Bgra8Unorm
        | wgpu::TextureFormat::Bgra8UnormSrgb => 4,
        wgpu::TextureFormat::Rgba16Float => 8,
        wgpu::TextureFormat::Rgba32Float => 16,
        other => bail!("env: cannot upload a cube map as {other:?}"),
    })
}

/// Upload the decoded KTX cube map so `irradiancecube.wgsl` and `prefilterenvmap.wgsl` can sample
/// it. One `write_texture` per mip: `pad_level_for_upload` already lays the six faces out back to
/// back with 256-byte-aligned rows, which is exactly the `D2`-array copy layout.
pub fn upload_source_cube_map(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &CubeMap,
) -> Result<wgpu::TextureView> {
    let bpp = bytes_per_texel(source.format)?;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("environment cube"),
        size: wgpu::Extent3d {
            width: source.width,
            height: source.height,
            depth_or_array_layers: 6,
        },
        mip_level_count: source.mip_levels(),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: source.format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (mip, level) in source.levels.iter().enumerate() {
        let padded = ktx::pad_level_for_upload(level, bpp);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ktx::padded_bytes_per_row(level.width, bpp) as u32),
                rows_per_image: Some(level.height),
            },
            wgpu::Extent3d {
                width: level.width,
                height: level.height,
                depth_or_array_layers: 6,
            },
        );
    }

    Ok(texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("environment cube"),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    }))
}

/// One of the two cube generators: `filtercube.vert` plus one fragment stage, drawn six times per
/// mip into the matching face/mip view of the target cube.
struct CubeFacePass {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    label: &'static str,
    target_format: wgpu::TextureFormat,
    clear_color: wgpu::Color,
}

impl CubeFacePass {
    fn new(
        device: &wgpu::Device,
        module: &wgpu::ShaderModule,
        label: &'static str,
        target_format: wgpu::TextureFormat,
        immediate_size: u32,
    ) -> Self {
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // The source is `Rgba16Float`, which is filterable - the C++ generator
                        // samples it with `VK_FILTER_LINEAR` too.
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(CubeMesh::vertex_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // `generateIrradianceCube` sets `CULL_MODE_NONE` (`pbrtexture.cpp:2666`): the cube
                // is viewed from inside, so culling would discard every face.
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: Default::default(),
            },
            // depthTest and depthWrite are `VK_FALSE` in both generators.
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_layout,
            label,
            target_format,
            // `clearValues.color = {0.0f, 0.0f, 0.2f, 0.0f}` (`pbrtexture.cpp:2616`, `:2995`).
            // `wgpu-hal/src/vulkan/conv.rs:223` passes a `wgpu::Color` straight through as
            // `VkClearColorValue::float32` for a float attachment, so 0.2 stays 0.2.
            clear_color: wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.2,
                a: 0.0,
            },
        }
    }

    /// Render every mip and face of `target`. `push(mip, face)` builds the immediate block for one
    /// draw, which is where the per-mip roughness of the prefilter pass enters.
    fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        target: &wgpu::Texture,
        cube: &CubeMesh,
        push: &dyn Fn(u32, u32) -> [u8; CUBE_PUSH_SIZE],
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(self.label),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let dim = target.width();
        let mips = target.mip_level_count();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(self.label),
        });
        for mip in 0..mips {
            // `viewport = dim * pow(0.5, mip)` (`pbrtexture.cpp:2690`).
            let side = (dim as f32 * 0.5f32.powi(mip as i32)).max(1.0) as u32;
            for face in 0..6u32 {
                let view = target.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(self.label),
                    format: Some(self.target_format),
                    // A `D2` view of exactly one layer of the 2D-array that backs the cube.
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    usage: Some(wgpu::TextureUsages::RENDER_ATTACHMENT),
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    base_array_layer: face,
                    array_layer_count: Some(1),
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(self.label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(self.clear_color),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.set_vertex_buffer(0, cube.vertex_buffer.slice(..));
                pass.set_index_buffer(cube.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_immediates(0, &push(mip, face));
                pass.set_viewport(0.0, 0.0, side as f32, side as f32, 0.0, 1.0);
                pass.draw_indexed(0..cube.index_count, 0, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }
}

/// `genbrdflut.vert`/`.frag`: a fullscreen triangle, no bindings, `Rg16Float` output.
fn brdf_pipeline(device: &wgpu::Device, module: &wgpu::ShaderModule) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("brdf lut"),
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("brdf lut"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            // Three vertices derived from `@builtin(vertex_index)`, so no vertex buffer.
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: Default::default(),
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rg16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The generated environment maps plus the sampler they are all bound with, as views ready to be
/// dropped straight into the `shading` and `skybox` bind groups.
pub struct EnvMaps {
    /// The uploaded KTX cube - `textures.environmentCube` in C++, and what `skybox.wgsl` samples.
    pub source: wgpu::TextureView,
    /// 64^3, [`mip_count`](`IRRADIANCE_DIM`) mips - `irradiancecube.wgsl` output. `Rgba16Float`
    /// rather than C++'s `Rgba32Float`, see the module docs.
    pub irradiance: wgpu::TextureView,
    /// 512^3, [`mip_count`](`PREFILTER_DIM`) mips, `Rgba16Float` - `prefilterenvmap.wgsl` output.
    pub prefilter: wgpu::TextureView,
    /// 512^2, `Rg16Float` - `genbrdflut.wgsl` output.
    pub brdf_lut: wgpu::TextureView,
    /// One sampler for all four, matching the C++ `VK_FILTER_LINEAR` / `CLAMP_TO_EDGE` cube sampler
    /// (`base/VulkanTexture.cpp:646-662`) and the 2D LUT sampler.
    pub sampler: wgpu::Sampler,
}

impl EnvMaps {
    /// Build the generated maps. `source` is the decoded KTX cube map, `cube` the mesh drawn per
    /// face (positions only - the WGSL face shaders use `inPos` as a direction, never a normal).
    ///
    /// Every pass is submitted as it is built, so this must run on the queue that renders the scene
    /// and before any frame reads the generated maps.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        modules: &[wgpu::ShaderModule],
        source: &CubeMap,
        cube: &CubeMesh,
    ) -> Result<Self> {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("env"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            lod_min_clamp: 0.0,
            // The C++ cube sampler uses `maxLod = mipLevels`; wgpu clamps to the bound texture's
            // own level count, which is what `lod_max_clamp = INFINITY` (the default) means here.
            ..Default::default()
        });

        let source_view = upload_source_cube_map(device, queue, source)?;

        let irradiance = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("irradiance cube"),
            size: wgpu::Extent3d {
                width: IRRADIANCE_DIM,
                height: IRRADIANCE_DIM,
                depth_or_array_layers: 6,
            },
            mip_level_count: mip_count(IRRADIANCE_DIM),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let prefilter = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("prefilter cube"),
            size: wgpu::Extent3d {
                width: PREFILTER_DIM,
                height: PREFILTER_DIM,
                depth_or_array_layers: 6,
            },
            mip_level_count: mip_count(PREFILTER_DIM),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let brdf_lut = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("brdf lut"),
            size: wgpu::Extent3d {
                width: BRDF_DIM,
                height: BRDF_DIM,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        // The two cube generators share one vertex shader (`filtercube.vert`) and one vertex
        // buffer, and differ only in fragment stage, output format and immediate contents.
        let irradiance_pass = CubeFacePass::new(
            device,
            &modules[shaders::IRRADIANCE_CUBE],
            "irradiance cube",
            wgpu::TextureFormat::Rgba16Float,
            IRRADIANCE_IMMEDIATE_SIZE,
        );
        let prefilter_pass = CubeFacePass::new(
            device,
            &modules[shaders::PREFILTER_ENV_MAP],
            "prefilter cube",
            wgpu::TextureFormat::Rgba16Float,
            PREFILTER_IMMEDIATE_SIZE,
        );

        let projection = cube_projection();
        let views = face_view_matrices();
        // `mvp = perspective(PI/2, 1, 0.1, 512) * matrices[f]` - the same for every mip.
        irradiance_pass.run(
            device,
            queue,
            &source_view,
            &sampler,
            &irradiance,
            cube,
            &|_mip, face| irradiance_immediates(projection * views[face as usize]),
        );
        prefilter_pass.run(
            device,
            queue,
            &source_view,
            &sampler,
            &prefilter,
            cube,
            &|mip, face| {
                prefilter_immediates(
                    projection * views[face as usize],
                    prefilter_roughness(mip),
                )
            },
        );

        let lut_view = brdf_lut.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("generate brdf lut"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("brdf lut"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &lut_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // `clearValues.color = {0.0f, 0.0f, 0.0f, 1.0f}` (`pbrtexture.cpp:2340`).
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&brdf_pipeline(device, &modules[shaders::GEN_BRDF_LUT]));
                // Three vertices, no vertex buffer - `genbrdflut.vert` derives the position from
                // `gl_VertexIndex`, and the WGSL port does the same from `@builtin(vertex_index)`.
                pass.draw(0..3, 0..1);
            }
            queue.submit([encoder.finish()]);
        }

        Ok(Self {
            source: source_view,
            irradiance: irradiance.create_view(&wgpu::TextureViewDescriptor {
                label: Some("irradiance cube"),
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            }),
            prefilter: prefilter.create_view(&wgpu::TextureViewDescriptor {
                label: Some("prefilter cube"),
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            }),
            brdf_lut: lut_view,
            sampler,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_counts_match_the_cpp_log2() {
        assert_eq!(mip_count(IRRADIANCE_DIM), 7);
        assert_eq!(mip_count(PREFILTER_DIM), 10);
        // `floor(log2(1920)) + 1 == 10 + 1` - `pbrtexture.cpp:3763`.
        assert_eq!(hiz_mip_levels(1920, 1080), 11);
    }

    #[test]
    fn immediate_blocks_have_the_push_block_sizes() {
        assert_eq!(IRRADIANCE_IMMEDIATE_SIZE, 72);
        assert_eq!(PREFILTER_IMMEDIATE_SIZE, 72);
        // `set_immediates` requires a multiple of `IMMEDIATE_DATA_ALIGNMENT` (4).
        assert_eq!(CUBE_PUSH_SIZE as u32 % 4, 0);
    }

    #[test]
    fn irradiance_immediates_encode_the_sampling_deltas() {
        let mvp = Mat4::from_cols_array(&[2.0; 16]);
        let b = irradiance_immediates(mvp);
        assert_eq!(f32_at(&b, 0), 2.0);
        assert_eq!(f32_at(&b, 64), std::f32::consts::TAU / 180.0);
        assert_eq!(f32_at(&b, 68), std::f32::consts::FRAC_PI_2 / 64.0);
    }

    #[test]
    fn prefilter_immediates_encode_roughness_and_sample_count() {
        let b = prefilter_immediates(Mat4::IDENTITY, 3.0 / 9.0);
        assert_eq!(f32_at(&b, 64), 3.0 / 9.0);
        assert_eq!(
            u32_at(&b, 68),
            PREFILTER_NUM_SAMPLES,
            "numSamples must stay 32 - prefilterenvmap.wgsl loops over it"
        );
    }

    #[test]
    fn prefilter_roughness_spans_zero_to_one() {
        // `roughness = mip / (numMips - 1)`, so the last mip is exactly 1.0 and matches
        // `MAX_REFLECTION_LOD` in common.wgsl.
        assert_eq!(prefilter_roughness(0), 0.0);
        assert_eq!(prefilter_roughness(9), 1.0);
        assert_eq!(prefilter_roughness(3), 3.0 / 9.0);
    }

    #[test]
    fn face_matrices_are_the_six_glm_rotations() {
        let m = face_view_matrices();
        // These are *view* matrices (world -> camera), so the world direction a face looks along is
        // `view^-1 * -Z`, not `view * -Z`.
        let expected = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, -1.0),
        ];
        for (view, want) in m.iter().zip(expected) {
            let dir = view.inverse().transform_point3(Vec3::NEG_Z);
            let dir = Vec3::new(round(dir.x), round(dir.y), round(dir.z));
            assert_eq!(dir, want, "a face view points the wrong way");
            // Rotation only: no scale, no translation.
            assert_eq!(round(view.w_axis.x), 0.0);
            assert_eq!(round(view.w_axis.y), 0.0);
            assert_eq!(round(view.w_axis.z), 0.0);
            assert_eq!(round(view.w_axis.w), 1.0);
        }
    }

    #[test]
    fn cube_projection_is_the_cpp_perspective() {
        let p = cube_projection();
        // perspective(PI/2, 1, 0.1, 512): y scale = 1/tan(PI/4) = 1, x scale = 1/aspect = 1.
        // The y scale is negated by `flip_projection_y` - see the function's docs.
        assert_eq!(round(p.x_axis.x), 1.0);
        assert_eq!(round(p.y_axis.y), -1.0);
        // Zero-to-one depth, right-handed: `z_axis.w == -1` and
        // `w_axis.z == near*far/(near-far)`, which is exactly what `glm::perspective` produces
        // under `GLM_FORCE_DEPTH_ZERO_TO_ONE`.
        assert_eq!(round(p.z_axis.w), -1.0);
        let want = 0.1 * 512.0 / (0.1 - 512.0);
        assert!((p.w_axis.z - want).abs() < 1e-5, "w_axis.z = {}", p.w_axis.z);
        assert_eq!(round(p.w_axis.w), 0.0);
    }

    #[test]
    fn cube_mesh_is_the_gltf_cube_transformed_like_the_loader() {
        // 36 indices into 24 positions, and the mesh still spans +/-5 after the node rotation: a
        // rotation cannot change the set of axis-aligned extremes of a centred cube.
        assert_eq!(CUBE_INDICES.len(), 36);
        assert!(CUBE_INDICES.iter().all(|&i| (i as usize) < CUBE_POSITIONS.len()));
        // The node quaternion in `cube.gltf` is stored as float32 (0.70710683 / 0.70710671), so a
        // rotated coordinate is 4.999999 rather than exactly 5 - compare with a tolerance.
        for p in cube_positions_transformed() {
            assert!(
                p.iter().all(|v| (v.abs() - 5.0).abs() < 1e-3),
                "a rotated cube vertex left the +/-5 shell: {p:?}"
            );
        }
        // The node quaternion is a 90 degree rotation about X, so Y and Z swap, and the loader's
        // `FlipY` then negates Y: vertex 0 goes (-5, -5, -5) -> (-5, +5, -5).
        let p0 = cube_positions_transformed()[0];
        assert!(
            (p0[0] + 5.0).abs() < 1e-3 && (p0[1] - 5.0).abs() < 1e-3 && (p0[2] + 5.0).abs() < 1e-3,
            "unexpected first cube vertex: {p0:?}"
        );
    }

    fn round(v: f32) -> f32 {
        (v * 1e6).round() / 1e6
    }

    fn f32_at(b: &[u8], at: usize) -> f32 {
        f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }
}
