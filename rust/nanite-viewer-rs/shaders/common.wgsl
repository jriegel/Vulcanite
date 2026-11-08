// Shared structs and helper functions for the nanite compute shaders.
//
// Every struct here mirrors a `std430` GLSL buffer in
// `shaders/glsl/pbrtexture/` byte for byte, so the same storage buffers can be
// bound to either implementation. WGSL's `vec3<f32>` follows the same
// align-16 / size-12 rule as `glm::vec3`, which is what makes the 112-byte
// `BvhNodeInfo` stride work out identically; the explicit `_padN` members keep
// the offsets obvious.
//
// Counter fields that a shader increments with `atomicAdd` are declared
// `atomic<u32>`; a shader that only *reads* the same buffer binds it through
// the matching non-atomic struct (`CulledClustersAtomic`, `SwrNumVertices`, ...).
// The bytes on both sides are identical.

struct BvhNodeInfo {
  clusterIntervals: vec2<i32>,
  _pad0: u32,
  pMinWorld: vec3<f32>,
  _pad1: u32,
  pMaxWorld: vec3<f32>,
  objectId: u32,
  errorR: vec4<f32>,
  errorRP: vec4<f32>,
  errorWorld: vec2<f32>,
  _pad2: vec2<f32>,
  childrenNodeIndices: vec4<i32>,
};

struct ClusterInfo {
  pMinWorld: vec3<f32>,
  _pad0: u32,
  pMaxWorld: vec3<f32>,
  triangleIndicesStart: u32,
  triangleIndicesEnd: u32,
  objectIdx: u32,
  _pad1: u32,
};

struct ErrorInfo {
  errorWorld: vec2<f32>,
  _pad0: vec2<f32>,
  centerR: vec4<f32>,
  centerRP: vec4<f32>,
};

// `vkglTF::Vertex`, 112 bytes.
struct Vertex {
  pos: vec3<f32>,
  _pad0: u32,
  normal: vec3<f32>,
  _pad1: u32,
  uv: vec2<f32>,
  _pad2: vec2<f32>,
  color: vec4<f32>,
  joint0: vec4<f32>,
  weight0: vec4<f32>,
  tangent: vec3<f32>,
  _pad3: u32,
};

// The `ClusterIndices` block of `bvhtraversal.comp` / `error.comp` /
// `culling.comp`, written version.
struct CulledClusters {
  clusterSize: atomic<u32>,
  frustumCullingNum: atomic<u32>,
  occulusionCullingNum: atomic<u32>,
  errorCullingNum: atomic<u32>,
  clusters: array<u32>,
};

// Same bytes, read-only view.
struct CulledClustersAtomic {
  culledClusterSize: u32,
  frustumCullingNum: u32,
  occulusionCullingNum: u32,
  errorCullingNum: u32,
  culledClusterIndices: array<u32>,
};

struct CurrBVHNodes {
  currBvhNodeInfoSize: u32,
  currBVHNodeInfoIndices: array<u32>,
};

struct NextBVHNodes {
  nextBvhNodeInfoSize: atomic<u32>,
  nextBVHNodeInfoIndices: array<u32>,
};

// `UBOMats` of `bvhtraversal.comp` / `culling.comp` (binding 4) and
// `UBOCullingMatrices` - five `mat4`s, 320 bytes, see `abi::SIZE_CULLING_MATRICES`.
struct CullingMatrices {
  model: mat4x4<f32>,
  lastView: mat4x4<f32>,
  lastProj: mat4x4<f32>,
  currView: mat4x4<f32>,
  currProj: mat4x4<f32>,
};

// `UBOMats` of `bvhtraversal.comp` (binding 7) and `error.comp` (binding 2):
// `mat4 view; mat4 proj; vec3 camUp; vec3 camRight;` - 160 bytes.
// The pads are scalars on purpose: a `vec3` member is 16-byte aligned, so padding with one would
// slide `camRight` to 160 instead of the 144 that `std140` gives it.
struct ErrorMatrices {
  view: mat4x4<f32>,
  proj: mat4x4<f32>,
  camUp: vec3<f32>,
  _pad0: f32,
  camRight: vec3<f32>,
  _pad1: f32,
};

struct DrawInfo {
  num_indices: u32,
  _pad0: u32,
  _pad1: u32,
  _pad2: u32,
};

// `UBO` of `swrasterize.comp` (binding 6): `mat4 proj; mat4 model; mat4 view; vec3 camPos;` - 208
// bytes. Same shape as `UboMatrices`, and the same buffer backs both.
struct DrawMatrices {
  proj: mat4x4<f32>,
  model: mat4x4<f32>,
  view: mat4x4<f32>,
  camPos: vec3<f32>,
  _pad0: f32,
};

// `numVertices_hw` of `culling.comp` (written) and `DrawIndexedIndirect` args.
struct HwrDrawIndirect {
  indexCount: atomic<u32>,
  instanceCount: u32,
  firstIndex: u32,
  vertexOffset: u32,
  firstInstance: u32,
};

// `swrDispatch` of `clearimage.comp`: written once by invocation (0,0) and then
// read as an indirect dispatch. Nothing increments it, so no field is atomic.
struct SwrDispatch {
  x: u32,
  y: u32,
  z: u32,
};

struct SwrNumVertices {
  indexCount: u32,
};

struct SwrNumVerticesAtomic {
  indexCount: atomic<u32>,
};

// Screen-space AABB of a bounding box, in normalised [0,1] framebuffer coords:
// `screenXY.xy` = min, `screenXY.zw` = max, plus the minimum depth.
struct ScreenAABB {
  screenXY: vec4<f32>,
  minZ: f32,
};

// NDC -> window coordinates (top-left origin, [0,1]).
//
// The GLSL original is `hpos * vec4(0.5, 0.5, 1, 1) + vec4(0.5, 0.5, 0, 0)`, which is right for
// the C++ example because it draws with a **positive-height** viewport
// (`base/VulkanInitializers.hpp:168-181`, a plain assignment) and an unflipped projection
// (`base/camera.hpp:61`, `flipY = false`): NDC y = **-1** is framebuffer row 0 there.
//
// wgpu's Vulkan backend installs `height: -rect.h` (`wgpu-hal-30.0.1/src/vulkan/command.rs:1028`),
// so NDC y = **+1** is row 0. Combined with [`crate::camera::flip_projection_y`], which already
// negated the projection's y row, the extra negation here *cancels* it, and the resulting row is
// numerically identical to the C++ one. That is not cosmetic: the HiZ pyramid is built from
// `fragCoord`-driven passes, and the hardware rasterizer lands where the flipped viewport puts it,
// so a compute pass that used the GLSL mapping straight would address the mirrored row - which is
// exactly the "scattered fragments outside the silhouette plus internal holes" this fixes.
fn windowFromNdc(v: vec4<f32>) -> vec4<f32> {
  return vec4<f32>(v.x * 0.5 + 0.5, 0.5 - v.y * 0.5, v.z, v.w);
};

// `getScreenAABB` of `bvhtraversal.comp` / `culling.comp`. The eight corners are
// walked with bit tests instead of the GLSL version's eight named corners.
fn getScreenAABB(bMin: vec3<f32>, bMax: vec3<f32>, lastView: mat4x4<f32>, lastProj: mat4x4<f32>) -> ScreenAABB {
  var minXY = vec2<f32>(1.0, 1.0);
  var maxXY = vec2<f32>(0.0, 0.0);
  var minZ = 1.0;
  for (var i: i32 = 0; i < 8; i = i + 1) {
    let x = select(bMin.x, bMax.x, (i & 1) != 0);
    let y = select(bMin.y, bMax.y, (i & 2) != 0);
    let z = select(bMin.z, bMax.z, (i & 4) != 0);
    var hpos = lastProj * lastView * vec4<f32>(x, y, z, 1.0);
    hpos = hpos / hpos.w;
    hpos = windowFromNdc(hpos);
    minXY = min(minXY, hpos.xy);
    maxXY = max(maxXY, hpos.xy);
    minZ = min(minZ, hpos.z);
  }
  return ScreenAABB(vec4<f32>(minXY, maxXY), minZ);
}

// `frustumCulling` of `bvhtraversal.comp` / `frustrumCulling` of `culling.comp`.
// Returns true when the box is *outside* the frustum.
fn frustumCulling(bMin: vec3<f32>, bMax: vec3<f32>, currView: mat4x4<f32>, currProj: mat4x4<f32>) -> bool {
  let eps = 1e-3;
  var inFrustum = false;
  for (var i: i32 = 0; i < 8; i = i + 1) {
    let x = select(bMin.x, bMax.x, (i & 1) != 0);
    let y = select(bMin.y, bMax.y, (i & 2) != 0);
    let z = select(bMin.z, bMax.z, (i & 4) != 0);
    let hpos = currProj * currView * vec4<f32>(x, y, z, 1.0);
    if (hpos.w == 0.0) {
      return false;
    }
    let c = hpos.xyz / hpos.w;
    let inside = c.x > -1.0 - eps && c.x < 1.0 + eps
      && c.y > -1.0 - eps && c.y < 1.0 + eps
      && c.z > 0.0 - eps && c.z < 1.0 + eps;
    inFrustum = inFrustum || inside;
  }
  return !inFrustum;
}

// `hizTexel`: the texel `textureLod(lastHZB, uv, level)` picks with the C++
// `hizbuffer.sampler` (`pbrtexture.cpp:3799`, `VK_FILTER_NEAREST` +
// `VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE`), i.e. `floor(uv * size)` clamped into
// the mip. `textureLoad` has no addressing mode, so the clamp is explicit.
fn hizTexel(uv: vec2<f32>, dims: vec2<f32>) -> vec2<i32> {
  return vec2<i32>(
    i32(clamp(floor(uv.x * dims.x), 0.0, dims.x - 1.0)),
    i32(clamp(floor(uv.y * dims.y), 0.0, dims.y - 1.0)));
}

// `occlusionCulling` of `bvhtraversal.comp` / `culling.comp`.
//
// The GLSL original samples with `textureLod` through a **nearest** sampler, so the
// four taps are pure texel fetches and there is nothing to filter. This port uses
// `textureLoad` instead of `textureSampleLevel` for exactly that reason: a WGSL
// `sampler` is a *filtering* one, and wgpu then rejects a non-filterable float
// texture - `R32Float` is non-filterable unless `Features::FLOAT32_FILTERABLE` is
// enabled. Dropping the sampler drops that requirement.
// `screenSize` is `textureSize(lastHZB, 0).xy` (`vec2<f32>(width, height)`).
fn occlusionCulling(aabb: ScreenAABB, lastHZB: texture_2d<f32>, screenSize: vec2<f32>) -> bool {
  let clipXY = aabb.screenXY;
  let pixelXY = clipXY * vec4<f32>(screenSize.x, screenSize.y, screenSize.x, screenSize.y);
  let pixelSpan = pixelXY.zw - pixelXY.xy;
  var hzbLevel = ceil(log2(max(pixelSpan.x, pixelSpan.y)));
  let hzbLevel_1 = max(hzbLevel - 1.0, 0.0);
  let texScale = exp2(-hzbLevel_1);
  let texSpan = ceil(pixelXY.zw * texScale) - floor(pixelXY.xy * texScale);
  if (texSpan.x < 2.0 && texSpan.y < 2.0) {
    hzbLevel = hzbLevel_1;
  }
  // Vulkan clamps the `textureLod` level to the sampler's `maxLod` *and* to the
  // image's mip count (`pbrtexture.cpp:3805` sets `maxLod = mipLevels`), so a
  // request above the top mip silently becomes the top mip. `hzbLevel` is
  // `ceil(log2(pixelSpan))`, which reaches `mipLevels` exactly when a node spans
  // the whole screen (`ceil(log2(640)) == 10` for a 10-level pyramid), so the
  // clamp is not defensive - it is hit by the root node of every model.
  // `textureLoad` with an out-of-range level has no defined result, and in
  // practice it reads 0, which makes `minZ > maxHiz` cull the entire tree.
  let level = i32(clamp(hzbLevel, 0.0, f32(textureNumLevels(lastHZB) - 1u)));
  let dims = vec2<f32>(textureDimensions(lastHZB, level));
  let z1 = textureLoad(lastHZB, hizTexel(vec2<f32>(clipXY.x, clipXY.y), dims), level).x;
  let z2 = textureLoad(lastHZB, hizTexel(vec2<f32>(clipXY.z, clipXY.y), dims), level).x;
  let z3 = textureLoad(lastHZB, hizTexel(vec2<f32>(clipXY.x, clipXY.w), dims), level).x;
  let z4 = textureLoad(lastHZB, hizTexel(vec2<f32>(clipXY.z, clipXY.w), dims), level).x;
  let maxHiz = max(max(z1, z2), max(z3, z4));
  return aabb.minZ > maxHiz;
}

// `getScreenBoundRadiusSq` of `bvhtraversal.comp` / `error.comp` / `culling.comp`:
// the squared radius of the bounding sphere in *pixels*, squared again by the
// callers' error test. Note this returns pixels^2, which is why the threshold is
// a pixel-count squared (`thresholdInt / 1e6`).
fn getScreenBoundRadiusSq(center: vec3<f32>, r: f32, view: mat4x4<f32>, proj: mat4x4<f32>, camUp: vec3<f32>, camRight: vec3<f32>, screenSize: vec2<f32>) -> f32 {
  let m = proj * view;
  var c = m * vec4<f32>(center, 1.0);
  c = c / c.w;
  c = c * vec4<f32>(0.5, 0.5, 1.0, 1.0) + vec4<f32>(0.5, 0.5, 0.0, 0.0);
  var p0 = m * vec4<f32>(r * camUp + center, 1.0);
  p0 = p0 / p0.w;
  p0 = p0 * vec4<f32>(0.5, 0.5, 1.0, 1.0) + vec4<f32>(0.5, 0.5, 0.0, 0.0);
  var p1 = m * vec4<f32>(r * camRight + center, 1.0);
  p1 = p1 / p1.w;
  p1 = p1 * vec4<f32>(0.5, 0.5, 1.0, 1.0) + vec4<f32>(0.5, 0.5, 0.0, 0.0);
  let cc = c.xy * screenSize;
  let v0 = p0.xy * screenSize - cc;
  let v1 = p1.xy * screenSize - cc;
  // Deliberately *not* [`windowFromNdc`]: this returns a squared pixel *distance*, and mirroring
  // all three points negates their y differences, which the dot product squares away. The result
  // is identical either way.
  return max(dot(v0, v0), dot(v1, v1));
}

// The visibility buffer's 64-bit id: `depth:32 | clusterId:15 | objectId:11 |
// triangleId:6`, see `packPixel` in `swrasterize.comp` / `merger.comp`.
fn packVis(depth: f32, clusterId: u32, objectId: u32, triangleId: u32) -> u64 {
  let d = 1.0 - depth;
  return (u64(bitcast<u32>(d)) << 32u)
    | (u64(clusterId & 0x7FFFu) << 17u)
    | (u64(objectId & 0x7FFu) << 6u)
    | u64(triangleId & 0x3Fu);
}

// The 32-bit half the hardware-rasterized pass stores in `HWRVisBuffer`.
fn packVis32(clusterId: u32, objectId: u32, triangleId: u32) -> u32 {
  return ((clusterId & 0x7FFFu) << 17u) | ((objectId & 0x7FFu) << 6u) | (triangleId & 0x3Fu);
}

// ---------------------------------------------------------------------------
// Visibility-buffer decode, mirroring the unpacking every GLSL reader repeats.
// ---------------------------------------------------------------------------

fn visDepth64(v: u64) -> f32 {
  return 1.0 - bitcast<f32>(u32(v >> 32u));
}

fn visDepth32(v: u32) -> f32 {
  return 1.0 - bitcast<f32>(v >> 17u << 15u);
}

// WGSL has **no 64-bit integer literals** (the spec gives `i64`/`u64` no literal form), so every
// 64-bit mask is a converted `u32` literal. `0x7FFFu64` does not even lex.
fn visClusterId64(v: u64) -> u32 {
  return u32((v >> 17u) & u64(0x7FFFu));
}

fn visClusterId32(v: u32) -> u32 {
  return (v >> 17u) & 0x7FFFu;
}

fn visObjectId64(v: u64) -> u32 {
  return u32((v >> 6u) & u64(0x7FFu));
}

fn visObjectId32(v: u32) -> u32 {
  return (v >> 6u) & 0x7FFu;
}

fn visTriangleId64(v: u64) -> u32 {
  return u32(v & u64(0x3Fu));
}

fn visTriangleId32(v: u32) -> u32 {
  return v & 0x3Fu;
}

// The id the hardware pass carries through the rasterizer: `packVis32` without
// the depth, i.e. exactly what `hwrasterize.geom` writes to `HWRVisBuffer`.
fn unpackHwrId(id: u32) -> vec3<u32> {
  return vec3<u32>(visObjectId32(id), visClusterId32(id), visTriangleId32(id));
}

// ---------------------------------------------------------------------------
// Tone mapping / PBR helpers, shared by `skybox.frag`, `pbrtexture.frag` and
// `shading.frag`. All three include the same `base/pbr.glsl` + tonemap code.
// ---------------------------------------------------------------------------

// `UBOPa` of `base/pbr.glsl` (72 bytes, see `abi::encode_ubo_params`).
struct UboParams {
  // Four point lights: position (xyz) + intensity (w).
  lights: array<vec4<f32>, 4>,
  exposure: f32,
  gamma: f32,
  _pad0: f32,
  _pad1: f32,
};

// `Uncharted2Tonemap` of `shading.frag` / `pbrtexture.frag` / `skybox.frag`
// (http://filmicgames.com/archives/75).
fn uncharted2Tonemap(x: vec3<f32>) -> vec3<f32> {
  let A = 0.15;
  let B = 0.50;
  let C = 0.10;
  let D = 0.20;
  let E = 0.02;
  let F = 0.30;
  return ((x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F)) - E / F;
}

// The full tone map + gamma step every display shader ends with. `whitePoint`
// is 11.2 in the C++ viewer.
fn tonemapAndGamma(color: vec3<f32>, exposure: f32, gamma: f32) -> vec3<f32> {
  var color2 = uncharted2Tonemap(color * exposure);
  color2 = color2 * (1.0 / uncharted2Tonemap(vec3<f32>(11.2)));
  return pow(color2, vec3<f32>(1.0 / gamma));
}

// ---------------------------------------------------------------------------
// The object UBO of the hardware-rasterize pass and the skybox pass.
// `uniformBuffers.object` / `uniformBuffers.skybox` in pbrtexture.cpp; 144 bytes,
// see `abi::SIZE_UBO_MATRICES`.
// ---------------------------------------------------------------------------

struct UboMatrices {
  projection: mat4x4<f32>,
  model: mat4x4<f32>,
  view: mat4x4<f32>,
  camPos: vec3<f32>,
  _pad0: f32,
};

// The `UBO` of `shading.frag`: `mat4 invView; mat4 invProj; vec3 camPos;` - 144
// bytes, see `abi::SIZE_SHADING_UBO`.
struct UboShading {
  invView: mat4x4<f32>,
  invProj: mat4x4<f32>,
  camPos: vec3<f32>,
  _pad0: f32,
};

// The two cube-map generators (`generateIrradianceCube` / `generatePrefilteredCube`)
// both push `mat4 mvp` at offset 0 plus two scalars at 64 and 68, but the scalars
// are `float deltaPhi/deltaTheta` in one and `float roughness/uint numSamples` in
// the other, so each of them declares its own immediate-block struct.

// ---------------------------------------------------------------------------
// `base/pbr.glsl`, verbatim.
// ---------------------------------------------------------------------------

const PI: f32 = 3.1415926535897932384626433832795;

fn D_GGX(dotNH: f32, roughness: f32) -> f32 {
  let alpha = roughness * roughness;
  let alpha2 = alpha * alpha;
  let denom = dotNH * dotNH * (alpha2 - 1.0) + 1.0;
  return alpha2 / (PI * denom * denom);
}

fn G_SchlicksmithGGX(dotNL: f32, dotNV: f32, roughness: f32) -> f32 {
  let r = roughness + 1.0;
  let k = (r * r) / 8.0;
  let GL = dotNL / (dotNL * (1.0 - k) + k);
  let GV = dotNV / (dotNV * (1.0 - k) + k);
  return GL * GV;
}

fn F_Schlick(cosTheta: f32, F0: vec3<f32>) -> vec3<f32> {
  return F0 + (vec3<f32>(1.0) - F0) * pow(1.0 - cosTheta, 5.0);
}

fn F_SchlickR(cosTheta: f32, F0: vec3<f32>, roughness: f32) -> vec3<f32> {
  return F0 + (max(vec3<f32>(1.0 - roughness), F0) - F0) * pow(1.0 - cosTheta, 5.0);
}

// `prefilteredReflection` of `shading.frag`. MAX_REFLECTION_LOD is the number of
// mips of the prefiltered cube minus one (512 px -> 10 mips -> 9.0).
const MAX_REFLECTION_LOD: f32 = 9.0;

fn prefilteredReflection(
  R: vec3<f32>,
  roughness: f32,
  prefilteredMap: texture_cube<f32>,
  lodSampler: sampler,
) -> vec3<f32> {
  let lod = roughness * MAX_REFLECTION_LOD;
  let lodf = floor(lod);
  let lodc = ceil(lod);
  let a = textureSampleLevel(prefilteredMap, lodSampler, R, lodf);
  let b = textureSampleLevel(prefilteredMap, lodSampler, R, lodc);
  return mix(a, b, lod - lodf).rgb;
}

// `specularContribution` of `base/pbr.glsl`, with the light colour fixed to
// white as `shading.frag` does.
fn specularContribution(
  L: vec3<f32>,
  V: vec3<f32>,
  N: vec3<f32>,
  F0: vec3<f32>,
  metallic: f32,
  roughness: f32,
  ALBEDO: vec3<f32>,
) -> vec3<f32> {
  // Direction of half vector
  let H = normalize(V + L);
  let dotNH = clamp(dot(N, H), 0.0, 1.0);
  let dotNV = clamp(dot(N, V), 0.0, 1.0);
  let dotNL = clamp(dot(N, L), 0.0, 1.0);
  var color = vec3<f32>(0.0);
  let lightColor = vec3<f32>(1.0);
  if (dotNL > 0.0) {
    // D = Normal distribution (Distribution of the microfacets)
    let D = D_GGX(dotNH, roughness);
    // G = Geometric shadowing term (Microfacets shadowing)
    let G = G_SchlicksmithGGX(dotNL, dotNV, roughness);
    // F = Fresnel term (Reflectance as a function of incident angle and refractive index)
    let F = F_Schlick(dotNV, F0);
    let spec = D * F * G / (4.0 * dotNL * dotNV + 0.001);
    let kD = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    color += (kD * ALBEDO / PI + spec) * dotNL;
  }
  return color;
}

// `getBarycentricCoord` of `shading.frag`.
fn getBarycentricCoord(center: vec3<f32>, v0: vec3<f32>, v1: vec3<f32>, v2: vec3<f32>) -> vec3<f32> {
  let v0v1 = v1 - v0;
  let v0v2 = v2 - v0;
  let area2 = length(cross(v0v1, v0v2));
  var bary = vec3<f32>(0.0);
  bary.x = length(cross(v2 - v1, center - v1)) / area2;
  bary.y = length(cross(v0 - v2, center - v2)) / area2;
  bary.z = 1.0 - bary.x - bary.y;
  return bary;
}

// ---------------------------------------------------------------------------
// GGX importance sampling, shared by `genbrdflut.frag` and
// `prefilterenvmap.frag`. Both GLSL originals define their own copies with the
// same body; keeping one here is what makes the BRDF LUT and the prefiltered
// cube agree.
// ---------------------------------------------------------------------------

// Based on http://byteblacksmith.com/improvements-to-the-canonical-one-liner-glsl-rand-for-opengl-es-2.0/
fn envRandom(co: vec2<f32>) -> f32 {
  let a = 12.9898;
  let b = 78.233;
  let c = 43758.5453;
  let dt = dot(co.xy, vec2<f32>(a, b));
  // GLSL `mod(x, y)`; WGSL has no fmod builtin.
  let sn = dt - 3.14 * floor(dt / 3.14);
  return fract(sin(sn) * c);
}

// Radical inverse based on http://holger.dammertz.org/stuff/notes_HammersleyOnHemisphere.html
fn hammersley2d(i: u32, N: u32) -> vec2<f32> {
  var bits = (i << 16u) | (i >> 16u);
  bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
  bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
  bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
  bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
  let rdi = f32(bits) * 2.3283064365386963e-10;
  return vec2<f32>(f32(i) / f32(N), rdi);
}

// Based on Karis' SIGGRAPH 2013 course notes.
fn importanceSample_GGX(Xi: vec2<f32>, roughness: f32, normal: vec3<f32>) -> vec3<f32> {
  // Maps a 2D point to a hemisphere with spread based on roughness
  let alpha = roughness * roughness;
  let phi = 2.0 * PI * Xi.x + envRandom(normal.xz) * 0.1;
  let cosTheta = sqrt((1.0 - Xi.y) / (1.0 + (alpha * alpha - 1.0) * Xi.y));
  let sinTheta = sqrt(1.0 - cosTheta * cosTheta);
  let H = vec3<f32>(sinTheta * cos(phi), sinTheta * sin(phi), cosTheta);

  // Tangent space
  let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(normal.z) < 0.999);
  let tangentX = normalize(cross(up, normal));
  let tangentY = normalize(cross(normal, tangentX));

  // Convert to world space
  return normalize(tangentX * H.x + tangentY * H.y + normal * H.z);
}

// `hashUIntToVec4` of `shading.frag`: four independent prime multiplies, each
// reduced to its low byte and pushed through `exp`.
fn hashUIntToVec4(value: u32) -> vec4<f32> {
  let h1 = value * 2654435761u;
  let h2 = value * 2246822519u;
  let h3 = value * 3266489917u;
  let h4 = value * 668265263u;
  let scale = 4.0 / 255.0;
  return vec4<f32>(
    exp(-scale * f32(255u - (h1 & 0xFFu))),
    exp(-scale * f32(255u - (h2 & 0xFFu))),
    exp(-scale * f32(255u - (h3 & 0xFFu))),
    exp(-scale * f32(255u - (h4 & 0xFFu))),
  );
}
