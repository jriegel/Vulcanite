//! A Rust replacement for `examples/pbrtexture/pbrtexture.cpp`.
//!
//! The C++ viewer is a Vulkan sample built on Sascha Willems' framework; this crate rebuilds the
//! same render graph on **wgpu** with an **egui** overlay and **WGSL** shaders. It is deliberately
//! not a rewrite of the algorithm: every buffer, binding, push constant, dispatch count and pipeline
//! state below mirrors the C++ one, because the whole point is that the two viewers show the same
//! image from the same `.fcc` file.
//!
//! What it can load:
//! - a baked `<model>.fcc` ([`nanite_blob::read`]), with no glTF involved at all, and
//! - a `.gltf`/`.glb` baked in-process through the Rust asset pipeline
//!   ([`nanite_bake_rs::bake_asset`]), which produces the same in-memory [`scene::Scene`].
//!
//! The frame graph, in the order `buildCommandBuffers` records it
//! (`pbrtexture.cpp:440-1100`):
//!
//! ```text
//! HZB -> SHADER_READ          (wgpu: implicit by resource state)
//! copy initNodeInfoIndices -> currNodeInfoIndices
//! zero the 5 culled-cluster counters
//! for each BVH depth level:   bvhtraversal.comp   (ping-pong curr/next)
//! error.comp                  (world -> screen error per cluster)
//! culling.comp                (frustum + HZB + LOD -> hw index list, sw index list)
//! clearimage.comp             (clear the swr visibility buffer, size the indirect dispatch)
//! dispatchIndirect            swrasterize.comp    (64-bit visibility buffer)
//! render pass                 hwrasterize         (32-bit id into the HW vis buffer)
//! merger.comp                 (hw vs sw -> finalDepth / finalVis)
//! render pass                 skybox, then the fullscreen shading triangle
//! depthcopy.comp              finalDepth -> HZB level 0
//! for each HZB mip:           genhiz.comp
//! ```
//!
//! Four things differ from Vulkan by necessity, and all are noted where they matter:
//! - the HW raster pass has no geometry shader, so `culling.wgsl` also writes a per-vertex index
//!   list and the vertex shader reads it through `@builtin(vertex_index)`;
//! - the BRDF LUT is `Rg16Float`, which wgpu does not allow as a storage texture, so it is generated
//!   by a render pass rather than a compute shader;
//! - no view of a depth image may be *bound* in wgpu: `texture_use_parameters`
//!   (`wgpu-core-30/src/device/resource.rs`) rejects any view whose aspects contain DEPTH with
//!   `Error::DepthStencilAspect`, before the `BindingType::Texture { sample_type: Depth }` case is
//!   ever reached. `merger.comp`'s `textureLoad(HWRZBuffer, ..)` therefore has no wgpu equivalent,
//!   and `hwrasterize.wgsl` writes the fragment depth to a second `R32Float` colour attachment of
//!   the same pass instead (see [`renderer`]);
//! - there is **no HiZ sampler at all**. `bvhtraversal.wgsl` and `culling.wgsl` tap `lastHZB` with
//!   `textureLoad(texel, level)`, where the texel comes from `floor(uv * size)` clamped into the mip
//!   - the C++ `hizbuffer.sampler` is `VK_FILTER_NEAREST` + `CLAMP_TO_EDGE`, so that is the same four
//!   texels. This matters because wgpu derives a texture binding's `sample_type` from whether a
//!   *sampler* is used in the entry point (`wgpu-core-30/src/validation.rs`), and a `Float` sample
//!   type with `filterable: true` requires the bound view to be filterable too; `R32Float` is not.
//!   With `textureLoad` the derived type is `UnfilterableFloat`, which `R32Float` satisfies, so
//!   nothing in this crate needs `Features::FLOAT32_FILTERABLE`.

pub mod abi;
pub mod app;
pub mod camera;
pub mod capture;
pub mod env;
pub mod ktx;
pub mod load;
pub mod renderer;
pub mod scene;
pub mod shaders;
