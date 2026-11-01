#pragma once
#include "VulkanglTFModel.h"
#include <OpenMesh/Core/IO/MeshIO.hh>
#include <OpenMesh/Core/Mesh/TriMesh_ArrayKernelT.hh>
#include <tinygltf/tiny_gltf.h>

#include <filesystem>

#include "Common.h"
#include "Mesh.h"
#include "NaniteBlob.h"

struct NaniteMesh {
	// The glTF flags every consumer of the builder must use. `vkglTF::Model::loadFromFile`
	// rewrites `vertexBuffer` in place - `FlipY` negates pos.y/normal.y and
	// `PreTransformVertices` applies the node hierarchy matrix - and the builder consumes
		// exactly those vertices, so a loader that uses a different set bakes a mirrored or
		// misplaced mesh that still passes every blob hash check (the source hash only covers
		// the file bytes). Both `pbrtexture.cpp` and `tools/nanite_bake` use this constant.
	static constexpr uint32_t kBuilderLoadingFlags =
		vkglTF::FileLoadingFlags::PreTransformVertices
		| vkglTF::FileLoadingFlags::PreMultiplyVertexColors
		| vkglTF::FileLoadingFlags::FlipY;

	// How the binary blob cache (NaniteBlob.h) is used by `initNaniteInfo`.
	enum class BlobMode {
		Auto,    // prefer a blob, else the JSON cache, else build; always bake after a build
		JsonOnly,// legacy path: JSON cache / build only, never read or write a blob
		Require, // blob only - fail if it is missing or stale, proving the viewer path ran
		Rebuild, // ignore every cache, rebuild from the glTF and re-bake the blob
	};

	uint32_t lodNums = 0;
	glm::mat4 modelMatrix;
	std::vector<Mesh> meshes;
	OpenMesh::HPropHandleT<int32_t> clusterGroupIndexPropHandle;

	/************ Load Mesh *************/
	// Load from vkglTF
	const vkglTF::Model * vkglTFModel;
	const vkglTF::Mesh* vkglTFMesh;
	void setModelPath(const char* path) { filepath = path; };
	void loadvkglTFModel(const vkglTF::Model& model);
	void vkglTFMeshToOpenMesh(MyMesh& mymesh, const vkglTF::Mesh& mesh);
	void vkglTFPrimitiveToOpenMesh(MyMesh& mymesh, const vkglTF::Primitive& prim);
	// TODO: Load from tinygltf
	const tinygltf::Model* tinyglTFModel;
	const tinygltf::Mesh* tinyglTFMesh;
	void loadglTFModel(const tinygltf::Model& model); // TODO
	void glTFMeshToOpenMesh(MyMesh& mymesh, const tinygltf::Mesh& mesh); // TODO

	/************ Process DAG *************/
	std::vector<ClusterNode> flattenedClusterNodes;
	void flattenDAG();

	/************ Flatten BVH *************/
	std::shared_ptr<NaniteBVHNode> virtualBVHRootNode;
	std::vector<NaniteBVHNodeInfo> flattenedBVHNodeInfos;
	void flattenBVH();

	/************ Build Info *************/
	void generateNaniteInfo();

	std::vector<ClusterInfo> clusterInfo;
	std::vector<ErrorInfo> errorInfo;
	std::vector<uint32_t> sortedClusterIndices;
	void buildClusterInfo();

	/************ Serialization *************/
	// Returns false if the cache could not be written (read-only target directory, disk full,
	// ...). Caching is optional, so callers must be able to continue without it.
	bool serialize(const std::string& filepath);
	void deserialize(const std::string& filepath);

	/************ Binary asset cache (asset pipeline -> viewer) *************/
	// The final GPU payloads, ready to be memcpy'd into storage buffers. Only filled
	// when the mesh was loaded from a blob (see `blobLoaded`).
	NaniteBlob::Asset blob;
	bool blobLoaded = false;

	// `<model>_naniteCache\\` next to the source mesh; holds the JSON cache and the LOD meshes.
	static std::string cachePathFor(const std::string& filepath);
	// `<model>.fcc` next to the source mesh. The blob is deliberately a *sibling* of the mesh
	// rather than a file inside the cache directory: it is the deliverable of the asset
	// pipeline, so it can be copied, shipped, or opened by the viewer on its own.
	static std::string blobPathFor(const std::string& filepath);
	// True for a path the viewer can open directly instead of a glTF.
	static bool isBlobPath(const std::string& path);
	// Bake the GPU payloads produced by the builder into the binary cache.
	void bakeBlob(const std::string& filepath);
	// Load them back. Returns false if the blob is missing, stale or incompatible.
	bool loadBlob(const std::string& filepath, bool strictSourceHash = false);
	// Load a blob from an explicit path. `sourceMesh` is the glTF the caller has loaded: it is
	// what a strict hash check re-reads. Left empty, the blob's own recorded source path is used
	// instead - that is what makes a lone `.fcc` loadable even when its mesh is gone.
	bool loadBlobFile(const std::string& blobPath, const std::string& sourceMesh = std::string(), bool strictSourceHash = false);

	void initNaniteInfo(const std::string& filepath, bool useCache = true, BlobMode blobMode = BlobMode::Auto);
	// Blob-only entry point: reaches the same state `initNaniteInfo` would, but from a `.fcc`
	// alone - no glTF, no OpenMesh, no METIS, no JSON. `blobPath` may be a `.gltf`/`.glb`, in
	// which case its `<model>.fcc` sibling is what gets loaded.
	bool initNaniteInfoFromBlob(const std::string& blobPath, bool strictSourceHash = true);
	/*
		What data structure should be used to store 
		1. lods
		2. cluster of each lods
		3. cluster group (no need)
	
	*/
	vks::VulkanDevice* device;
	const vkglTF::Model * model;
	vkglTF::Model::Vertices vertices;
	vkglTF::Model::Indices indices;
	std::vector<uint32_t> indexBuffer;
	std::vector<vkglTF::Vertex> vertexBuffer;
	std::vector<vkglTF::Primitive> primitives;
	std::vector<uint32_t> clusterIndexOffset; // This offset is caused by different LODs

	const char* filepath = nullptr;
	const char* cache_time_key = "cache_time";

	std::vector<Mesh> debugMeshes;
	void checkDeserializationResult(const std::string& filepath);

	bool operator==(const NaniteMesh & other) const {
		if (meshes.size() != other.meshes.size()) return false;
		for (int i = 0; i < meshes.size(); i++)
		{
			if (meshes[i].mesh.n_vertices() != other.meshes[i].mesh.n_vertices()) return false;
			if (meshes[i].mesh.n_faces() != other.meshes[i].mesh.n_faces()) return false;
		}
		return true;
	}
};

void loadvkglTFModel(const vkglTF::Model& model, std::vector<NaniteMesh>& naniteMeshes);

void packNaniteMeshesToIndexBuffer(const std::vector<NaniteMesh>& naniteMeshes, std::vector<uint32_t>& indexBuffer);