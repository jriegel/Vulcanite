// Port of `shaders/glsl/pbrtexture/merger.comp`: resolve the hardware pass
// (R32Uint id + Depth32Float depth) against the software pass (R64Uint
// depth|id) into the final R32Uint visibility buffer and R32Float depth buffer
// the shading pass reads.

struct Immediates {
  visClusters: i32,
};

var<immediate> pcs: Immediates;

// 0: the hardware pass's depth, as an R32Float color attachment written by
// `hwrasterize.wgsl`'s second fragment output. In Vulkan `merger.comp` samples `HWRZBuffer`
// itself; wgpu 30 refuses to bind any view whose aspects contain DEPTH
// (`wgpu-core::device::resource::texture_use_parameters` -> `Error::DepthStencilAspect`), so the
// depth has to reach the merger through a colour attachment of the very same render pass. Same
// values, same pass, no extra copy.
@group(0) @binding(0) var hwrDepthBuffer: texture_storage_2d<r32float, read>;
// 1: HWRVisBuffer.
@group(0) @binding(1) var hwrVisBuffer: texture_storage_2d<r32uint, read>;
// 2: the software rasterizer's combined depth+id buffer.
@group(0) @binding(2) var swrDepthVisBuffer: texture_storage_2d<r64uint, read>;
// 3/4: the merged output the shading pass reads.
@group(0) @binding(3) var finalDepthBuffer: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var finalVisBuffer: texture_storage_2d<r32uint, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let screenSize = vec2<u32>(textureDimensions(hwrVisBuffer));
  let index = vec2<i32>(i32(gid.x), i32(gid.y));
  if (gid.x >= screenSize.x || gid.y >= screenSize.y) {
    return;
  }
  // The C++ original samples the depth at the pixel centre with a LINEAR filter,
  // which resolves to exactly this texel; `textureLoad` says the same thing
  // without a sampler or a filtering question.
  let hwrDepth = textureLoad(hwrDepthBuffer, index).x;
  let hwrVis = textureLoad(hwrVisBuffer, index).x;
  let swrDepthVis = textureLoad(swrDepthVisBuffer, index).x;
  // WGSL has no 64-bit integer literals, hence the `u64(...)` conversions here.
  let swrValid = swrDepthVis != u64(0u);
  let hwrValid = hwrVis != 0xFFFFFFFFu;
  let swrDepth = select(1.0, visDepth64(swrDepthVis), swrValid);
  let swrVis = select(0xFFFFFFFFu, u32(swrDepthVis & u64(0xFFFFFFFFu)), swrValid);
  let finalDepth = min(swrDepth, hwrDepth);
  // Mode 3 replaces the id with a pure occlusion result: 1 where the software
  // pass won, 0 where the hardware pass won, 0xFFFFFFFF where nothing covered.
  var finalVis: u32;
  if (pcs.visClusters != 3) {
    finalVis = select(hwrVis, swrVis, swrDepth < hwrDepth);
  } else {
    finalVis = select(0xFFFFFFFFu, select(0u, 1u, swrDepth < hwrDepth), swrValid || hwrValid);
  }
  textureStore(finalDepthBuffer, index, vec4<f32>(finalDepth, 0.0, 0.0, 0.0));
  textureStore(finalVisBuffer, index, vec4<u32>(finalVis, 0u, 0u, 0u));
}
