// Port of `shaders/glsl/pbrtexture/clearimage.comp`: zero the software
// rasterizer's 64-bit visibility buffer and, from one lucky invocation, turn the
// triangle count the culling shader produced into an indirect dispatch.
//
// The dispatch trick is why this pass exists at all: `swrasterize` is launched
// with `dispatchWorkgroupsIndirect`, so the count has to be computed on the GPU
// between culling and rasterizing.

@group(0) @binding(0) var swrDepthVisBuffer: texture_storage_2d<r64uint, write>;
@group(0) @binding(1) var<storage, read> numVerticesSw: SwrNumVertices;
@group(0) @binding(2) var<storage, read_write> swrDispatch: SwrDispatch;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  // WGSL never converts `u32` to `i32` implicitly, not even inside a composite constructor.
  let index = vec2<i32>(i32(gid.x), i32(gid.y));
  if (gid.x == 0u && gid.y == 0u) {
    // One workgroup per 32 indices, rounded up — matches
    // `(indexCount/3 + 31) / 32` of the original.
    swrDispatch.x = (numVerticesSw.indexCount / 3u + 31u) / 32u;
    swrDispatch.y = 1u;
    swrDispatch.z = 1u;
  }
  let screenSize = vec2<u32>(textureDimensions(swrDepthVisBuffer));
  if (gid.x >= screenSize.x || gid.y >= screenSize.y) {
    return;
  }
  // A `u64` composite cannot be built from `u32` literals — WGSL has no 64-bit literals and no
  // implicit widening, so each component is converted.
  textureStore(swrDepthVisBuffer, index, vec4<u64>(u64(0u), u64(0u), u64(0u), u64(0u)));
}
