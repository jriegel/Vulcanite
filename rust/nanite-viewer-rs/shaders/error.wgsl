// Port of `shaders/glsl/pbrtexture/error.comp`: project each *surviving*
// cluster's LOD error into screen space, so `culling.wgsl` can compare it
// against the threshold in pixels.
//
// Indexed by invocation id (i.e. by slot in the culled-cluster list), **not** by
// cluster index — `culling.wgsl` reads `projectedError[gid.x]` the same way.

struct Immediates {
  numClusters: i32,
  screenSize: vec2<f32>,
};

var<immediate> pcs: Immediates;

@group(0) @binding(0) var<storage, read> idata: array<ErrorInfo>;
@group(0) @binding(1) var<storage, read_write> odata: array<vec2<f32>>;
@group(0) @binding(2) var<uniform> ubomats: ErrorMatrices;
@group(0) @binding(3) var<storage, read> culledClusters: CulledClustersAtomic;
@group(0) @binding(4) var<storage, read> clusterObjectIndices: array<u32>;
@group(0) @binding(5) var<storage, read> inModelMats: array<mat4x4<f32>>;

@compute @workgroup_size(32)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x >= culledClusters.culledClusterSize) {
    return;
  }
  let clusterIndex = culledClusters.culledClusterIndices[gid.x];
  let objectId = clusterObjectIndices[gid.x];
  let error = idata[clusterIndex];
  let modelMat = inModelMats[objectId];

  // The radius is scaled by the model matrix's first column, matching the
  // original's `length(inModelMats[objectId] * vec4(R, 0, 0, 0))`.
  let centerR = (modelMat * vec4<f32>(error.centerR.xyz, 1.0)).xyz;
  let radiusR = length(modelMat * vec4<f32>(error.centerR.w, 0.0, 0.0, 0.0));
  let centerRP = (modelMat * vec4<f32>(error.centerRP.xyz, 1.0)).xyz;
  let radiusRP = length(modelMat * vec4<f32>(error.centerRP.w, 0.0, 0.0, 0.0));

  odata[gid.x].x = error.errorWorld.x * getScreenBoundRadiusSq(
    centerR, radiusR, ubomats.view, ubomats.proj, ubomats.camUp, ubomats.camRight, pcs.screenSize);
  odata[gid.x].y = error.errorWorld.y * getScreenBoundRadiusSq(
    centerRP, radiusRP, ubomats.view, ubomats.proj, ubomats.camUp, ubomats.camRight, pcs.screenSize);
}
