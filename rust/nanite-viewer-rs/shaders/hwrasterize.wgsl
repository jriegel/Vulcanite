// hwrasterize.wgsl - the hardware-rasterization pass.
//
// Port of hwrasterize.vert + hwrasterize.geom + hwrasterize.frag.
//
// Vulkan has a geometry shader here and WGSL does not, so the triangle -> vertex expansion that
// `hwrasterize.geom` does is folded into `culling.wgsl`, which already loops over every emitted
// triangle: it writes the packed visibility id it computes for the geometry shader into
// `outVertexIdsHw` at the same three slots it writes the triangle indices into `outTrianglesHw`.
// That makes the output vertex stream self-describing, so this pass becomes a *non-indexed* draw
// over `outTrianglesHw` / `outVertexIdsHw`:
//
//   Vulkan: index buffer -> vertex buffer -> geometry shader (gl_PrimitiveIDIn -> inID[])
//   wgpu:   vertex_index -> outTrianglesHw[vi] -> inVertices[...]  and  outVertexIdsHw[vi]
//
// `@builtin(primitive_index)` is deliberately NOT used: naga 30 validates it as a *fragment*
// (or ray-hit / mesh-output) input only (`valid/interface.rs`: `matches!(stage, Fragment | AnyHit
// | ClosestHit)`), and its SPIR-V writer additionally demands `Capability::Geometry`. So the
// per-vertex id is the only way this port works at all.
//
// Bindings are the C++ "hwRast" set (0 modelMats, 1 HWRIDBuffer, 2 UBO) plus the scene vertex
// buffer, which the geometry shader read through the vertex input instead of a binding.

@group(0) @binding(0)
var<storage, read> inModelMats: array<mat4x4<f32>>;
@group(0) @binding(1)
var<storage, read> inVertexIds: array<u32>;
@group(0) @binding(2)
var<uniform> ubo: UboMatrices;
@group(0) @binding(3)
var<storage, read> inVertices: array<Vertex>;
@group(0) @binding(4)
var<storage, read> inTriangles: array<u32>;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) @interpolate(flat) id: u32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertexIndex: u32) -> VsOut {
  var out: VsOut;

  // The packed id culling.wgsl computed for this output vertex slot.
  let packedId = inVertexIds[vertexIndex];
  let objectId = visObjectId32(packedId);

  // outTrianglesHw is indexed by output vertex slot, inVertices by scene vertex index.
  let sceneVertexIndex = inTriangles[vertexIndex];
  let pos = inVertices[sceneVertexIndex].pos;

  // hwrasterize.geom: gl_Position = ubo.projection * ubo.view * model * vec4(inPos, 1.0). Note
  // that ubo.model is deliberately not applied - the per-instance matrix already contains it.
  let model = inModelMats[objectId];
  out.pos = ubo.projection * ubo.view * model * vec4<f32>(pos, 1.0);
  out.id = packedId;
  return out;
}

// hwrasterize.frag: "color.x = outID;" into the R32ui visibility buffer. Depth is written by the
// fixed-function stage, exactly as in Vulkan.
//
// `@location(1)` is *not* in the Vulkan original. wgpu 30 rejects binding a depth-aspect texture
// view at all (`wgpu-core::device::resource::texture_use_parameters` returns `DepthStencilAspect`
// for any view whose aspects contain DEPTH), so `merger.comp`'s `textureLoad(HWRZBuffer, ...)` has
// no wgpu equivalent. Instead the fragment stage hands `fragCoord.z` - the same value the depth
// test compares, already in the [0, 1] window space both APIs use - to a second, R32Float color
// attachment of the same pass, and the merger reads that. `R32Float` is legal as both a color
// attachment and a storage texture (`wgpu_types::texture::format` guarantees
// `attachment | storage | binding`), so no extra pass or copy is needed.
struct FsOut {
  @location(0) id: u32,
  @location(1) depth: f32,
};

@fragment
fn fs_main(
  @location(0) @interpolate(flat) id: u32,
  @builtin(position) fragCoord: vec4<f32>,
) -> FsOut {
  var out: FsOut;
  out.id = id;
  out.depth = fragCoord.z;
  return out;
}
