#pragma once

/*
	NaniteBlob - binary, mmap-friendly asset cache for the Nanite viewer.

	This is the output of the *asset pipeline* and the input of the *viewer*.
	It stores the final GPU payloads (already in the exact layout the std430
	storage buffers expect) rather than the builder's intermediate state, so
	the viewer can upload them with a straight memcpy and never links OpenMesh
	or METIS.

	Layout of the file:

		+---------------------------+
		| NaniteBlobHeader          |  fixed size, magic + version + hashes
		+---------------------------+
		| NaniteBlobSection x N     |  section table (key, count, offset, size)
		+---------------------------+
		| padding to 256 bytes      |
		+---------------------------+
		| section payloads          |  raw POD arrays, each 256-byte aligned
		+---------------------------+

	All offsets are absolute byte offsets from the start of the file.
	Sections may appear in any order and unknown sections are ignored, so the
	format stays forward compatible.
*/

#include <cstdint>
#include <cfloat>
#include <string>
#include <vector>

#include <glm/glm.hpp>

#include "Config.h"
#include "GpuTypes.h"

// Bump whenever the *meaning* of a section changes but its byte layout does not
// (e.g. the clustering algorithm changes). Bump NANITE_BLOB_VERSION instead when
// a struct's byte layout changes.
// 2: the blob became a standalone file (<model>.fcc) and gained SECTION_SOURCE_PATH, so it
//    can be loaded without the source mesh being present.
#define NANITE_BLOB_VERSION 2u
#define NANITE_BUILDER_VERSION 1u

#define NANITE_BLOB_MAGIC 0x4244414Eu  // 'NADB' = Nanite Data Blob

// File extension of a baked blob. It is a real asset file next to the source mesh
// (`bunny.gltf` -> `bunny.fcc`), not a scratch file inside the cache directory, so it can be
// shipped to a viewer that never runs the builder.
#define NANITE_BLOB_EXTENSION ".fcc"

namespace NaniteBlob
{
	// Identifies a payload array inside the blob.
	enum SectionKey : uint32_t
	{
		SECTION_VERTICES = 0,   // vkglTF::Vertex[]  - unique vertices, LOD0..LODn concatenated
		SECTION_INDICES = 1,    // uint32_t[]        - triangle vertex indices, cluster-sorted
		SECTION_CLUSTER_INFO = 2,   // ClusterInfo[]   - mesh-relative triangle ranges
		SECTION_ERROR_INFO = 3,     // ErrorInfo[]     - per-cluster projected error + spheres
		SECTION_BVH_NODES = 4,      // BlobBVHNode[]   - per-mesh flattened BVH (local child indices)
		SECTION_SORTED_CLUSTER_INDICES = 5, // uint32_t[] - leaf -> cluster, LOD-offset
		SECTION_LOD_CLUSTER_NUMS = 6,       // uint32_t[] - cluster count of each LOD
		// The root stored in SECTION_BVH_NODES is the builder's *virtual* root, whose
		// children are the per-LOD root nodes. There is one per LOD, so they do not fit
		// in the 4-wide `children` field of BlobBVHNode and are listed here instead.
		SECTION_BVH_ROOTS = 7,      // uint32_t[] - local indices of the virtual root's children
		// The mesh this blob was baked from, as UTF-8. Only informational: it lets a viewer
		// open a lone .fcc and still know (and optionally re-hash) its source. A blob stays
		// usable when that file is gone.
		SECTION_SOURCE_PATH = 8,    // char[] - source mesh path, no NUL terminator
		SECTION_COUNT
	};

	// On-disk BVH node. Deliberately a fixed-size POD: `NaniteBVHNodeInfo` holds a
	// std::vector of children and a 32-wide cluster index array that the viewer
	// never reads, so storing it verbatim would be both non-POD and wasteful.
	// The builder already asserts a node has at most 4 children.
	struct BlobBVHNode
	{
		alignas(16) glm::vec3 pMin = glm::vec3(FLT_MAX);
		alignas(16) glm::vec3 pMax = glm::vec3(-FLT_MAX);
		alignas(16) glm::vec4 parentBoundingSphere = glm::vec4(0.0f);
		alignas(16) glm::ivec4 children = glm::ivec4(-1);
		// normalizedlodError, parentNormalizedError. Stored as float because that
		// is the precision the GPU struct (BVHNodeInfo::errorWorld) uses anyway.
		alignas(8) glm::vec2 errorWorld = glm::vec2(-FLT_MAX);
		alignas(4) uint32_t start = 0; // [start, end) into SECTION_SORTED_CLUSTER_INDICES
		alignas(4) uint32_t end = 0;
		alignas(4) int32_t depth = 0;
		alignas(4) int32_t nodeStatus = 0; // NaniteBVHNodeStatus
		alignas(4) int32_t lodLevel = -1;
		alignas(4) int32_t index = -1; // index within this mesh's flattened BVH
	};

	static_assert(alignof(BlobBVHNode) == 16, "BlobBVHNode must stay 16-byte aligned");
	static_assert(sizeof(BlobBVHNode) == 96, "BlobBVHNode layout changed - bump NANITE_BLOB_VERSION");

	struct NaniteBlobHeader
	{
		uint32_t magic = NANITE_BLOB_MAGIC;
		uint32_t version = NANITE_BLOB_VERSION;
		uint32_t builderVersion = NANITE_BUILDER_VERSION;
		uint64_t sourceHash = 0;  // FNV-1a of the source mesh file bytes
		uint64_t configHash = 0;  // FNV-1a of the builder parameters (Config.h)
		uint32_t lodNums = 0;
		uint32_t sectionCount = 0;
		// Struct sizes recorded at write time so a stale blob is rejected rather
		// than silently misread.
		uint32_t sizeBVHNodeInfo = 0;
		uint32_t sizeClusterInfo = 0;
		uint32_t sizeErrorInfo = 0;
		uint32_t sizeVertex = 0;
		uint32_t sizeBlobBVHNode = 0;
		uint32_t pad = 0;
	};

	struct NaniteBlobSection
	{
		uint32_t key = 0;
		uint32_t count = 0;
		uint64_t offset = 0;
		uint64_t size = 0;
	};

	// Everything the viewer needs to upload a mesh without rebuilding it.
	struct Asset
	{
		std::string sourcePath;
		uint64_t sourceHash = 0;
		uint32_t lodNums = 0;
		// Byte size of one vertex element (sizeof(vkglTF::Vertex)). Stored so the
		// blob layer never has to know the vertex type.
		uint32_t vertexStride = 0;

		std::vector<uint8_t> vertices; // raw vertex bytes, vertexStride per element
		std::vector<uint32_t> indices;
		std::vector<uint8_t> clusterInfo; // raw ClusterInfo bytes
		std::vector<uint8_t> errorInfo;   // raw ErrorInfo bytes
		std::vector<BlobBVHNode> bvhNodes;
		std::vector<uint32_t> sortedClusterIndices;
		std::vector<uint32_t> lodClusterNums;
		std::vector<uint32_t> bvhRoots;

		// Convenience accessors that reinterpret the raw byte sections.
		const ClusterInfo* clusterInfos() const { return reinterpret_cast<const ClusterInfo*>(clusterInfo.data()); }
		const ErrorInfo* errorInfos() const { return reinterpret_cast<const ErrorInfo*>(errorInfo.data()); }
		size_t clusterCount() const { return clusterInfo.size() / sizeof(ClusterInfo); }
		size_t errorCount() const { return errorInfo.size() / sizeof(ErrorInfo); }
		size_t vertexCount() const { return vertexStride ? vertices.size() / vertexStride : 0; }
	};

	// FNV-1a, 64 bit. Used for the source/config invalidation hashes.
	uint64_t fnv1a(const void* data, size_t size, uint64_t seed = 0xcbf29ce484222325ull);
	uint64_t fnv1aFile(const std::string& path, uint64_t seed = 0xcbf29ce484222325ull);

	// Hash of every builder parameter that changes the produced data.
	uint64_t configHash();

	// Write / read the blob. Returns false and fills `error` on any mismatch.
	// `read` keeps whatever `asset.sourcePath` the caller set (that file is what a strict hash
	// check re-reads) and fills it from SECTION_SOURCE_PATH when the caller left it empty - which
	// is how a standalone .fcc knows where it came from.
	bool write(const std::string& path, const Asset& asset, uint64_t source, std::string& error);
	bool read(const std::string& path, Asset& asset, std::string& error, bool strictSourceHash = false);

	// True if a blob file is present at `path`. The contents are validated by `read`,
	// which reports the exact reason when a blob turns out to be unusable.
	bool exists(const std::string& path);
}
