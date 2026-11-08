// Port of `shaders/glsl/pbrtexture/bvhtraversal.comp`: one wavefront level of
// the BVH per dispatch, ping-ponging between `currBVHNodes` and `nextBVHNodes`.
//
// Bindings are the C++ set-0 bindings, minus the HiZ sampler: `lastHZB` is read
// with `textureLoad` (see `occlusionCulling` in `common.wgsl`), so the sampler
// slot the GLSL combined image sampler implies is simply gone.

struct Immediates {
  screenSize: vec2<f32>,
  threshold: f32,
};

var<immediate> pcs: Immediates;

@group(0) @binding(0) var<storage, read> bvhNodeInfos: array<BvhNodeInfo>;
@group(0) @binding(1) var<storage, read> currBVHNodes: CurrBVHNodes;
@group(0) @binding(2) var<storage, read_write> nextBVHNodes: NextBVHNodes;
@group(0) @binding(3) var<storage, read_write> clusterIndexBuffer: CulledClusters;
@group(0) @binding(4) var<uniform> ubomats: CullingMatrices;
@group(0) @binding(5) var lastHZB: texture_2d<f32>;
@group(0) @binding(6) var<uniform> ubomats2: ErrorMatrices;
@group(0) @binding(7) var<storage, read_write> clusterObjectIndices: array<u32>;
@group(0) @binding(8) var<storage, read> sortedClusterIndices: array<u32>;

fn errorCulling(b: BvhNodeInfo) -> bool {
  let err = b.errorWorld.y * getScreenBoundRadiusSq(
    b.errorRP.xyz, b.errorRP.w,
    ubomats2.view, ubomats2.proj,
    ubomats2.camUp, ubomats2.camRight, pcs.screenSize);
  return err <= pcs.threshold;
}

@compute @workgroup_size(32)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let numIndices = arrayLength(&currBVHNodes.currBVHNodeInfoIndices);
  if (gid.x >= numIndices || gid.x >= currBVHNodes.currBvhNodeInfoSize) {
    return;
  }
  let nodeIndex = currBVHNodes.currBVHNodeInfoIndices[gid.x];
  let b = bvhNodeInfos[nodeIndex];

  let aabb = getScreenAABB(b.pMinWorld, b.pMaxWorld, ubomats.lastView, ubomats.lastProj);
  if (frustumCulling(b.pMinWorld, b.pMaxWorld, ubomats.currView, ubomats.currProj)) {
    return;
  }
  let screenSize = vec2<f32>(textureDimensions(lastHZB));
  if (occlusionCulling(aabb, lastHZB, screenSize)) {
    return;
  }
  if (errorCulling(b)) {
    return;
  }

  let start = u32(max(b.clusterIntervals.x, 0));
  let end = u32(max(b.clusterIntervals.y, 0));
  let leafClusterSize = end - start;

  if (leafClusterSize != 0u) {
    // A leaf: its `clusterIntervals` are a range into `sortedClusterIndices`.
    let base = atomicAdd(&clusterIndexBuffer.clusterSize, leafClusterSize);
    for (var i: u32 = 0u; i < leafClusterSize; i = i + 1u) {
      clusterIndexBuffer.clusters[base + i] = sortedClusterIndices[start + i];
      clusterObjectIndices[base + i] = b.objectId;
    }
  } else {
    // An inner node: hand its children to the next level. The GLSL original
    // counts the *leading* non-`-1` entries and stops at the first `-1`.
    var childSize: u32 = 0u;
    for (var i: i32 = 0; i < 4; i = i + 1) {
      if (b.childrenNodeIndices[i] == -1) {
        break;
      }
      childSize = childSize + 1u;
    }
    if (childSize == 0u) {
      return;
    }
    let base = atomicAdd(&nextBVHNodes.nextBvhNodeInfoSize, childSize);
    for (var i: u32 = 0u; i < childSize; i = i + 1u) {
      nextBVHNodes.nextBVHNodeInfoIndices[base + i] = u32(b.childrenNodeIndices[i32(i)]);
    }
  }
}
