#pragma once

/*
	GpuTypes.h - the plain-old-data payloads that end up in Vulkan std430
	storage buffers and in the Nanite binary asset cache.

	Deliberately free of OpenMesh / METIS / tinygltf includes so the viewer can
	load a baked asset without linking the asset pipeline's dependencies.
*/

#include <cstddef>
#include <cstdint>

#include <glm/glm.hpp>

#include "Config.h"

// ---------------------------------------------------------------------------
// Layout contract
//
// BVHNodeInfo, ClusterInfo and ErrorInfo are:
//   - bound as std430 storage buffers by bvhtraversal.comp / culling.comp / error.comp
//   - written verbatim into the Nanite binary asset cache (see NaniteBlob.h)
// Any change to the members below is therefore an on-disk *and* GPU ABI break.
// These asserts make that break a compile error instead of silent corruption;
// when one fires, update the GLSL structs, bump NANITE_BLOB_VERSION and rebuild
// every cache.
// ---------------------------------------------------------------------------

//  Only store information that are useful for subsequent traversal
//      aabb
//      parent error
//      children indices(glm::ivec4)
//  Some meta information about bvh should also be stored (in a uniform buffer)
//      Node count of each level (At least we should know the node count of level 0 to initiate traversal)
struct BVHNodeInfo {
    BVHNodeInfo() : pMinWorld(FLT_MAX), pMaxWorld(-FLT_MAX), childrenNodeIndices(-1), errorWorld(FLT_MAX), clusterIntervals(-1)
    {
    }
    alignas(16) glm::ivec2 clusterIntervals = glm::ivec2(-1, -1); // [clusterIntervals.x, clusterIntervals.y) is the range of clusterIndices
    alignas(16) glm::vec3 pMinWorld = glm::vec3(FLT_MAX);
    alignas(16) glm::vec3 pMaxWorld = glm::vec3(-FLT_MAX);
    alignas(4) uint32_t objectId;
    alignas(16)  glm::vec4 errorR;//node error and parent error. Node error should be non-neccessary, kept for now
    alignas(16)  glm::vec4 errorRP;//node error and parent error. Node error should be non-neccessary, kept for now
    alignas(8)  glm::vec2 errorWorld;//node error and parent error. Node error should be non-neccessary, kept for now
    alignas(16) glm::ivec4 childrenNodeIndices;
};

//ClusterInfo for drawing
struct ClusterInfo {
    alignas(16) glm::vec3 pMinWorld = glm::vec3(FLT_MAX);
    alignas(16) glm::vec3 pMaxWorld = glm::vec3(-FLT_MAX);
    // [triangleIndicesStart, triangleIndicesEnd) is the range of triangleIndicesSortedByClusterIdx
    // left close, right open
    alignas(4) uint32_t triangleIndicesStart; // Used to index Mesh::triangleIndicesSortedByClusterIdx
    alignas(4) uint32_t triangleIndicesEnd; // Used to index Mesh::triangleIndicesSortedByClusterIdx
    alignas(4) uint32_t objectIdx;

    void mergeAABB(const glm::vec3& pMinOther, const glm::vec3& pMaxOther) {
        pMinWorld = glm::min(pMinWorld, pMinOther);
        pMaxWorld = glm::max(pMaxWorld, pMaxOther);
    };
};

struct ErrorInfo
{
    alignas(8)  glm::vec2 errorWorld;//node error and parent error in world
    alignas(16) glm::vec4 centerR;
    alignas(16) glm::vec4 centerRP;
};

static_assert(alignof(BVHNodeInfo) == 16, "BVHNodeInfo must stay 16-byte aligned");
static_assert(sizeof(BVHNodeInfo) == 112, "BVHNodeInfo layout changed - update shaders and bump the blob version");
static_assert(offsetof(BVHNodeInfo, clusterIntervals) == 0, "BVHNodeInfo::clusterIntervals moved");
static_assert(offsetof(BVHNodeInfo, pMinWorld) == 16, "BVHNodeInfo::pMinWorld moved");
static_assert(offsetof(BVHNodeInfo, pMaxWorld) == 32, "BVHNodeInfo::pMaxWorld moved");
static_assert(offsetof(BVHNodeInfo, objectId) == 44, "BVHNodeInfo::objectId moved");
static_assert(offsetof(BVHNodeInfo, errorR) == 48, "BVHNodeInfo::errorR moved");
static_assert(offsetof(BVHNodeInfo, errorRP) == 64, "BVHNodeInfo::errorRP moved");
static_assert(offsetof(BVHNodeInfo, errorWorld) == 80, "BVHNodeInfo::errorWorld moved");
static_assert(offsetof(BVHNodeInfo, childrenNodeIndices) == 96, "BVHNodeInfo::childrenNodeIndices moved");

static_assert(alignof(ClusterInfo) == 16, "ClusterInfo must stay 16-byte aligned");
static_assert(sizeof(ClusterInfo) == 48, "ClusterInfo layout changed - update shaders and bump the blob version");
static_assert(offsetof(ClusterInfo, pMinWorld) == 0, "ClusterInfo::pMinWorld moved");
static_assert(offsetof(ClusterInfo, pMaxWorld) == 16, "ClusterInfo::pMaxWorld moved");
static_assert(offsetof(ClusterInfo, triangleIndicesStart) == 28, "ClusterInfo::triangleIndicesStart moved");
static_assert(offsetof(ClusterInfo, triangleIndicesEnd) == 32, "ClusterInfo::triangleIndicesEnd moved");
static_assert(offsetof(ClusterInfo, objectIdx) == 36, "ClusterInfo::objectIdx moved");

static_assert(alignof(ErrorInfo) == 16, "ErrorInfo must stay 16-byte aligned");
static_assert(sizeof(ErrorInfo) == 48, "ErrorInfo layout changed - update shaders and bump the blob version");
static_assert(offsetof(ErrorInfo, errorWorld) == 0, "ErrorInfo::errorWorld moved");
static_assert(offsetof(ErrorInfo, centerR) == 16, "ErrorInfo::centerR moved");
static_assert(offsetof(ErrorInfo, centerRP) == 32, "ErrorInfo::centerRP moved");
