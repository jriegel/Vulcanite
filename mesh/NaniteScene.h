#pragma once

#include <map>

#include "Instance.h"

class NaniteScene {
public:
	std::vector<Instance> naniteObjects;
	std::vector<NaniteMesh> naniteMeshes;
	std::map<std::string, uint32_t> naniteMeshSceneIndices;
	std::vector<uint32_t> indexOffsets;
	std::vector<uint32_t> indexCounts;

	vkglTF::Model::Vertices vertices;
	vkglTF::Model::Indices indices;

	std::shared_ptr<NaniteBVHNode> virtualRootNode; // The root node that connects all instances' root nodes
	std::vector<BVHNodeInfo> bvhNodeInfos; // cleaned version
	std::vector<uint32_t> clusterIndexOffsets; 
	std::vector<uint32_t> depthCounts;
	std::vector<uint32_t> depthLeafCounts; // Just for stats, not in usage
	std::vector<uint32_t> initNodeInfoIndices;
	uint32_t maxDepthCounts = 0;

	std::vector<uint32_t> clusterIndexCounts;
	uint32_t maxClusterNum = 0;

	std::vector<uint32_t> sortedClusterIndices;

	uint32_t maxLodLevelNum = -1;

	uint32_t sceneIndicesCount = 0;
	uint32_t visibleIndicesCount = 0;

	std::vector<ClusterInfo> clusterInfo;
	std::vector<ErrorInfo> errorInfo;

	void addNaniteMesh(const std::string & meshName, NaniteMesh& naniteMesh) {
		//naniteMeshes[meshName] = naniteMesh;
	};

	// Which entry of `naniteMeshes` an instance refers to. Pointer identity first: an
	// instance may point at a mesh that lives outside the scene (the example keeps its own
	// `naniteMesh` member and stores a copy here). `NaniteMesh::operator==` only compares
	// the per-LOD OpenMesh meshes, which a blob-loaded mesh does not have, so blob meshes
	// are identified by their source instead.
	size_t meshIndexOf(const Instance& instance) const {
		for (size_t i = 0; i < naniteMeshes.size(); i++) {
			if (&naniteMeshes[i] == instance.referenceMesh) {
				return i;
			}
		}
		for (size_t i = 0; i < naniteMeshes.size(); i++) {
			if (sameMesh(naniteMeshes[i], *instance.referenceMesh)) {
				return i;
			}
		}
		ASSERT(0, "instance references a mesh that is not part of the scene");
		return 0;
	}

	static bool sameMesh(const NaniteMesh& a, const NaniteMesh& b) {
		if (a.blobLoaded || b.blobLoaded) {
			return a.blobLoaded && b.blobLoaded
				&& a.blob.sourcePath == b.blob.sourcePath
				&& a.blob.sourceHash == b.blob.sourceHash
				&& a.lodNums == b.lodNums;
		}
		return a == b;
	}

	// True when every mesh in the scene came from a binary blob, i.e. the scene can be
	// linked entirely from baked payloads without touching OpenMesh or METIS.
	bool allMeshesBaked() const {
		if (naniteMeshes.empty()) {
			return false;
		}
		for (const auto& mesh : naniteMeshes) {
			if (!mesh.blobLoaded) {
				return false;
			}
		}
		return true;
	}

	Instance& addInstance(const std::string & meshName, glm::mat4 transform) {
		//Instance instance(&naniteMeshes[meshName], transform);
		//naniteObjects.push_back(instance);
		//return naniteObjects.back();
	}

	void createNaniteSceneInfo(vks::VulkanDevice* device, VkQueue transferQueue);

	// World space AABB of everything the scene can draw, i.e. of every instance's clusters with
	// `Instance::rootTransform` applied. This is the only bounds source that exists on *both*
	// paths: a blob-only load (`--model bunny.fcc`) never builds an OpenMesh mesh and deliberately
	// leaves `models.object` empty, so vkglTF's node dimensions are not available there.
	// Returns false when nothing has been uploaded yet.
	bool worldBounds(glm::vec3& pMin, glm::vec3& pMax) const;

	void createVertexIndexBuffer(vks::VulkanDevice* device, VkQueue transferQueue);
	void createClusterInfos(vks::VulkanDevice* device, VkQueue transferQueue);
	void createBVHNodeInfos(vks::VulkanDevice* device, VkQueue transferQueue);

	// Blob path for `createBVHNodeInfos`: links every instance's baked BVH into one flat
	// array by arithmetic only. It deliberately does *not* rebuild the shared_ptr tree
	// (`Instance::reconstructBVH`) only to flatten it again, which is what the generic
	// path pays for. The output must be identical to the generic path.
	void linkBlobBVHNodeInfos();
};