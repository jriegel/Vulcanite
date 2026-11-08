// Port of `shaders/glsl/pbrtexture/culling.comp`: for every cluster that
// survived BVH traversal, run frustum + HiZ occlusion + screen-space LOD error
// culling, then append its triangles to the hardware or software rasterizer's
// index buffer.
//
// Like the original there is **no bounds check** on the output buffers: they are
// sized to `sceneIndicesCount`, which is the exact upper bound. Do not add a
// check to "make it safe" — the C++ gotcha this mirrors is that undersized
// buffers silently wrote into the next allocation.

struct Immediates {
  numClusters: i32,
  threshold: f32,
  useFrustrumOcclusion: i32,
  useSoftwareRast: i32,
};

var<immediate> pcs: Immediates;

@group(0) @binding(0) var<storage, read> indata: array<ClusterInfo>;
@group(0) @binding(1) var<storage, read> inTriangles: array<u32>;
@group(0) @binding(2) var<storage, read_write> outTrianglesHw: array<u32>;
@group(0) @binding(3) var<storage, read_write> numVerticesHw: HwrDrawIndirect;
@group(0) @binding(4) var<uniform> ubomats: CullingMatrices;
@group(0) @binding(5) var lastHZB: texture_2d<f32>;
@group(0) @binding(7) var<storage, read> errorData: array<vec2<f32>>;
@group(0) @binding(8) var<storage, read_write> outIdsHw: array<vec3<u32>>;
@group(0) @binding(9) var<storage, read_write> outTrianglesSw: array<u32>;
@group(0) @binding(10) var<storage, read_write> outIdsSw: array<vec3<u32>>;
@group(0) @binding(11) var<storage, read_write> numVerticesSw: SwrNumVerticesAtomic;
@group(0) @binding(12) var<storage, read> culledClusters: CulledClustersAtomic;
@group(0) @binding(13) var<storage, read> clusterObjectIndices: array<u32>;
@group(0) @binding(14) var<storage, read> inModelMats: array<mat4x4<f32>>;
// `hwrasterize.geom` writes `HWRVisBuffer`'s id per fragment; wgpu has no geometry
// shader, so the id is produced here per *vertex* instead and read back by
// `hwrasterize.wgsl`'s vertex stage through `@builtin(vertex_index)`.
@group(0) @binding(15) var<storage, read_write> outVertexIdsHw: array<u32>;

@compute @workgroup_size(32)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x >= culledClusters.culledClusterSize) {
    return;
  }
  let clusterIndex = culledClusters.culledClusterIndices[gid.x];
  let objectId = clusterObjectIndices[gid.x];
  var culled = false;

  // Only the two defining corners are transformed, exactly as in the original.
  let modelMat = inModelMats[objectId];
  let currCluster = indata[clusterIndex];
  let bMin = (modelMat * vec4<f32>(currCluster.pMinWorld, 1.0)).xyz;
  let bMax = (modelMat * vec4<f32>(currCluster.pMaxWorld, 1.0)).xyz;

  let hizSize = vec2<f32>(textureDimensions(lastHZB));
  var pixelArea = 0.0;
  if (pcs.useFrustrumOcclusion == 1) {
    if (frustumCulling(bMin, bMax, ubomats.currView, ubomats.currProj)) {
      culled = true;
    }
    let aabb = getScreenAABB(bMin, bMax, ubomats.lastView, ubomats.lastProj);
    if (occlusionCulling(aabb, lastHZB, hizSize)) {
      culled = true;
    }
    let pixelXY = aabb.screenXY * vec4<f32>(hizSize.x, hizSize.y, hizSize.x, hizSize.y);
    let pixelSpan = pixelXY.zw - pixelXY.xy;
    pixelArea = pixelSpan.x * pixelSpan.y;
  } else {
    let aabb = getScreenAABB(bMin, bMax, ubomats.lastView, ubomats.lastProj);
    let pixelXY = aabb.screenXY * vec4<f32>(hizSize.x, hizSize.y, hizSize.x, hizSize.y);
    let pixelSpan = pixelXY.zw - pixelXY.xy;
    pixelArea = pixelSpan.x * pixelSpan.y;
  }

  // Small clusters go to the software rasterizer, big ones to the hardware one.
  let useSWR = (pcs.useSoftwareRast == 1) && pixelArea < 256.0;

  // Indexed by invocation id, not by cluster index — see `error.wgsl`.
  if (errorData[gid.x].y <= pcs.threshold || errorData[gid.x].x > pcs.threshold) {
    culled = true;
  }

  let totalVertices: u32 = select(
    0u,
    (currCluster.triangleIndicesEnd - currCluster.triangleIndicesStart) * 3u,
    !culled);

  var localIdx: u32 = 0u;
  if (useSWR) {
    localIdx = atomicAdd(&numVerticesSw.indexCount, totalVertices);
  } else {
    localIdx = atomicAdd(&numVerticesHw.indexCount, totalVertices);
  }
  if (culled) {
    return;
  }

  let numTriangles = totalVertices / 3u;
  for (var i: u32 = 0u; i < numTriangles; i = i + 1u) {
    let inIdx = currCluster.triangleIndicesStart * 3u + 3u * i;
    let outIdx = localIdx + 3u * i;
    if (useSWR) {
      outTrianglesSw[outIdx] = inTriangles[inIdx];
      outTrianglesSw[outIdx + 1u] = inTriangles[inIdx + 1u];
      outTrianglesSw[outIdx + 2u] = inTriangles[inIdx + 2u];
      outIdsSw[outIdx / 3u] = vec3<u32>(objectId, clusterIndex, i);
    } else {
      outTrianglesHw[outIdx] = inTriangles[inIdx];
      outTrianglesHw[outIdx + 1u] = inTriangles[inIdx + 1u];
      outTrianglesHw[outIdx + 2u] = inTriangles[inIdx + 2u];
      outIdsHw[outIdx / 3u] = vec3<u32>(objectId, clusterIndex, i);
      // The same id, once per vertex, for the geometry-shader-free hardware pass.
      let packedId = packVis32(clusterIndex, objectId, i);
      outVertexIdsHw[outIdx] = packedId;
      outVertexIdsHw[outIdx + 1u] = packedId;
      outVertexIdsHw[outIdx + 2u] = packedId;
    }
  }
}
