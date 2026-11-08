// shading.wgsl - the final shaded pass.
//
// Port of shading.vert + shading.frag, including all four *Visualize Clusters* modes.
//
// Bindings are the C++ "shading" set with the three combined image samplers split into a
// texture + sampler pair each (wgpu has no combined samplers), so bindings 8..10 become 8..13:
//
//   0 inCluster  1 inVertices  2 inTriangles  3 inModelMats
//   4 visBuffer (r32uint)      5 depthBuffer (r32float)
//   6 UboShading               7 UboParams
//   8 samplerIrradiance  9 its sampler  10 samplerBRDFLUT  11 its sampler
//   12 prefilteredMap    13 its sampler

@group(0) @binding(0)
var<storage, read> inCluster: array<ClusterInfo>;
@group(0) @binding(1)
var<storage, read> inVertices: array<Vertex>;
@group(0) @binding(2)
var<storage, read> inTriangles: array<u32>;
@group(0) @binding(3)
var<storage, read> inModelMats: array<mat4x4<f32>>;
@group(0) @binding(4)
var visBuffer: texture_storage_2d<r32uint, read>;
@group(0) @binding(5)
var depthBuffer: texture_storage_2d<r32float, read>;
@group(0) @binding(6)
var<uniform> ubo: UboShading;
@group(0) @binding(7)
var<uniform> uboParams: UboParams;
@group(0) @binding(8)
var samplerIrradiance: texture_cube<f32>;
@group(0) @binding(9)
var irradianceSampler: sampler;
@group(0) @binding(10)
var samplerBRDFLUT: texture_2d<f32>;
@group(0) @binding(11)
var brdfSampler: sampler;
@group(0) @binding(12)
var prefilteredMap: texture_cube<f32>;
@group(0) @binding(13)
var prefilteredSampler: sampler;

// `vis_clusters` from the C++ `RenderingPushConstants`. An immediate, *not* a binding: wgpu's
// derived pipeline layout sizes the immediate block from this declaration, and a bound
// `var<immediate>` is rejected by `derive_binding_type` (WrongBufferAddressSpace).
struct Immediates {
  visClusters: i32,
};
var<immediate> pcs: Immediates;

// shading.vert: a 3-vertex fullscreen triangle, no inputs. inUV is 0 or 2 at the corners, so
// `inUV * 2 - 1` spans NDC -1..3 across the screen.
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertexIndex: u32) -> VsOut {
  var out: VsOut;
  let uv = vec2<f32>(f32((vertexIndex << 1u) & 2u), f32(vertexIndex & 2u));
  out.uv = uv;
  out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
  return out;
}

@fragment
fn fs_main(@location(0) inUV: vec2<f32>, @builtin(position) fragCoord: vec4<f32>) -> @location(0) vec4<f32> {
  let screenPos = vec2<i32>(i32(fragCoord.x), i32(fragCoord.y));
  let ID = textureLoad(visBuffer, screenPos).x;
  if (ID == 0xFFFFFFFFu) {
    discard;
  }
  let depth = textureLoad(depthBuffer, screenPos).x;
  let clusterID = ID >> 17u;
  let triangleID = ID & 0x3Fu;
  let currCluster = inCluster[clusterID];
  let objectId = (ID >> 6u) & 0x7FFu;
  let globalTriangleID = currCluster.triangleIndicesStart + triangleID;
  let v0i = inTriangles[globalTriangleID * 3u + 0u];
  let v1i = inTriangles[globalTriangleID * 3u + 1u];
  let v2i = inTriangles[globalTriangleID * 3u + 2u];
  let ALBEDO = vec3<f32>(0.5);

  // Mode 2: per-triangle hash of the visibility buffer id.
  if (pcs.visClusters == 2) {
    let hashed = hashUIntToVec4(ID).xyz;
    return vec4<f32>(tonemapAndGamma(hashed, uboParams.exposure, uboParams.gamma), 1.0);
  }
  // Mode 1: LOD level, stored per vertex in `joint0.x` by the builder.
  if (pcs.visClusters == 1) {
    let lodLevel = inVertices[v0i].joint0.x;
    var c = vec3<f32>(0.0, 1.0, 1.0);
    if (lodLevel < 0.5) {
      c = vec3<f32>(1.0, 0.0, 0.0);
    } else if (lodLevel < 1.5) {
      c = vec3<f32>(0.0, 1.0, 0.0);
    } else if (lodLevel < 2.5) {
      c = vec3<f32>(0.0, 0.0, 1.0);
    } else if (lodLevel < 3.5) {
      c = vec3<f32>(1.0, 1.0, 0.0);
    } else if (lodLevel < 4.5) {
      c = vec3<f32>(1.0, 0.0, 1.0);
    }
    return vec4<f32>(c, 1.0);
  }
  // Mode 3: grayscale visibility buffer id.
  if (pcs.visClusters == 3) {
    return vec4<f32>(vec3<f32>(f32(ID) * 0.5 + 0.25), 1.0);
  }

  let ndc = vec4<f32>(inUV * 2.0 - 1.0, depth, 1.0);
  var worldPos = ubo.invView * ubo.invProj * ndc;
  worldPos = worldPos / worldPos.w;
  let model = inModelMats[objectId];
  let v0wpos = model * vec4<f32>(inVertices[v0i].pos, 1.0);
  let v1wpos = model * vec4<f32>(inVertices[v1i].pos, 1.0);
  let v2wpos = model * vec4<f32>(inVertices[v2i].pos, 1.0);
  let bary = getBarycentricCoord(worldPos.xyz, v0wpos.xyz, v1wpos.xyz, v2wpos.xyz);

  let v0nwpos = model * vec4<f32>(inVertices[v0i].normal, 0.0);
  let v1nwpos = model * vec4<f32>(inVertices[v1i].normal, 0.0);
  let v2nwpos = model * vec4<f32>(inVertices[v2i].normal, 0.0);
  let N = normalize(v0nwpos.xyz * bary.x + v1nwpos.xyz * bary.y + v2nwpos.xyz * bary.z);

  let V = normalize(ubo.camPos - worldPos.xyz);
  let R = reflect(-V, N);

  let metallic = 0.9;
  let roughness = 0.01;
  let F0 = mix(vec3<f32>(0.04), ALBEDO, metallic);

  var Lo = vec3<f32>(0.0);
  for (var i: i32 = 0; i < 4; i = i + 1) {
    let L = normalize(uboParams.lights[i].xyz - worldPos.xyz);
    Lo = Lo + specularContribution(L, V, N, F0, metallic, roughness, ALBEDO);
  }

  let brdf = textureSampleLevel(samplerBRDFLUT, brdfSampler, vec2<f32>(max(dot(N, V), 0.0), roughness), 0.0).rg;
  let reflection = prefilteredReflection(R, roughness, prefilteredMap, prefilteredSampler);
  let irradiance = textureSample(samplerIrradiance, irradianceSampler, N).rgb;

  // Diffuse based on irradiance
  let diffuse = irradiance * ALBEDO;

  let F = F_SchlickR(max(dot(N, V), 0.0), F0, roughness);

  // Specular reflectance
  let specular = reflection * (F * brdf.x + brdf.y);

  // Ambient part
  var kD = vec3<f32>(1.0) - F;
  kD = kD * (1.0 - metallic);
  let ambient = kD * diffuse + specular;
  let color = ambient + Lo;

  return vec4<f32>(tonemapAndGamma(color, uboParams.exposure, uboParams.gamma), 1.0);
}
