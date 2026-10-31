#include "NaniteScene.h"

#include <algorithm>
#include <queue>

void NaniteScene::createNaniteSceneInfo(vks::VulkanDevice* device, VkQueue transferQueue)
{
	createVertexIndexBuffer(device, transferQueue);
	createClusterInfos(device, transferQueue);
	if (allMeshesBaked()) {
		// Viewer path: link the baked BVHs arithmetically instead of rebuilding and
		// re-flattening a shared_ptr tree per instance.
		linkBlobBVHNodeInfos();
	}
	else {
		createBVHNodeInfos(device, transferQueue);
	}
    for (size_t i = 0; i < depthCounts.size(); i++)
    {
        std::cout << "Depth " << i << " has " << depthCounts[i] << " nodes." << std::endl;
        std::cout << "Depth " << i << " has " << depthLeafCounts[i] << " leaf nodes." << std::endl;
    }
    //ASSERT(0, "Stop here");
    std::cout << "Among each level, largest node count is: " << maxDepthCounts << std::endl;
    std::cout << "Total cluster count within current scene: " << maxClusterNum << std::endl;
}

void NaniteScene::createVertexIndexBuffer(vks::VulkanDevice* device, VkQueue transferQueue)
{
    std::vector<vkglTF::Vertex> vertexBuffer;
    std::vector<uint32_t> indexBuffer;

    int indexOffset = 0;
    indexOffsets.resize(naniteMeshes.size());
    indexCounts.resize(naniteMeshes.size());

    for (int i = 0; i < naniteMeshes.size(); ++i) {
        auto& naniteMesh = naniteMeshes[i];
        // Init vertex & index buffer
        auto& instance = Instance(&naniteMesh, glm::mat4(1.0f));
        if (naniteMesh.blobLoaded) {
            instance.initBufferFromBlob();
        }
        else {
            instance.initBufferForNaniteLODs();
        }
        vertexBuffer.insert(vertexBuffer.end(), instance.vertexBuffer.begin(), instance.vertexBuffer.end());
        for (auto index : instance.indexBuffer) {
            index += indexOffset;
            indexBuffer.push_back(index);
        }
        indexOffsets[i] = indexOffset;
        indexCounts[i] = instance.indexBuffer.size();
        indexOffset += instance.vertexBuffer.size();
        maxLodLevelNum = glm::max(maxLodLevelNum, instance.referenceMesh->lodNums);
    }

    size_t vertexBufferSize = vertexBuffer.size() * sizeof(vkglTF::Vertex);
    vertices.count = static_cast<uint32_t>(vertexBuffer.size());

    struct StagingBuffer {
        VkBuffer buffer;
        VkDeviceMemory memory;
    } vertexStaging, indexStaging;

    // Create staging buffers
// Vertex data
    VK_CHECK_RESULT(device->createBuffer(
        VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
        VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
        vertexBufferSize,
        &vertexStaging.buffer,
        &vertexStaging.memory,
        vertexBuffer.data()));

    // Create device local buffers
    // Vertex buffer
    VK_CHECK_RESULT(device->createBuffer(
        VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT | VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
        vertexBufferSize,
        &vertices.buffer,
        &vertices.memory));

    // Copy from staging buffers
    VkCommandBuffer copyCmd = device->createCommandBuffer(VK_COMMAND_BUFFER_LEVEL_PRIMARY, true);

    VkBufferCopy copyRegion = {};

    copyRegion.size = vertexBufferSize;
    vkCmdCopyBuffer(copyCmd, vertexStaging.buffer, vertices.buffer, 1, &copyRegion);

    device->flushCommandBuffer(copyCmd, transferQueue, true);

    vkDestroyBuffer(device->logicalDevice, vertexStaging.buffer, nullptr);
    vkFreeMemory(device->logicalDevice, vertexStaging.memory, nullptr);

    size_t indexBufferSize = indexBuffer.size() * sizeof(uint32_t);
    indices.count = static_cast<uint32_t>(indexBuffer.size());


    // Create staging buffers
    // Index data
    VK_CHECK_RESULT(device->createBuffer(
        VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
        VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT,
        indexBufferSize,
        &indexStaging.buffer,
        &indexStaging.memory,
        indexBuffer.data()));

    // Create device local buffers
    // Index buffer
    VK_CHECK_RESULT(device->createBuffer(
        VK_BUFFER_USAGE_INDEX_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_DST_BIT | VK_BUFFER_USAGE_STORAGE_BUFFER_BIT,
        VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
        indexBufferSize,
        &indices.buffer,
        &indices.memory));

    // Copy from staging buffers
    copyCmd = device->createCommandBuffer(VK_COMMAND_BUFFER_LEVEL_PRIMARY, true);

    copyRegion = {};

    copyRegion.size = indexBufferSize;
    vkCmdCopyBuffer(copyCmd, indexStaging.buffer, indices.buffer, 1, &copyRegion);

    device->flushCommandBuffer(copyCmd, transferQueue, true);

    vkDestroyBuffer(device->logicalDevice, indexStaging.buffer, nullptr);
    vkFreeMemory(device->logicalDevice, indexStaging.memory, nullptr);
}


void NaniteScene::createClusterInfos(vks::VulkanDevice* device, VkQueue transferQueue)
{
    sceneIndicesCount = 0;
    clusterIndexOffsets.resize(naniteMeshes.size());
    clusterIndexCounts.resize(naniteMeshes.size());
    for (size_t i = 0; i < naniteMeshes.size(); i++)
    {
        auto& naniteMesh = naniteMeshes[i];
        auto& instance = Instance(&naniteMesh, glm::mat4(1.0f));
        std::vector<ClusterInfo> meshClusterInfo;
        std::vector<ErrorInfo> meshErrorInfo;
        if (naniteMesh.blobLoaded) {
            // Baked payload: reinterpret the blob sections, no OpenMesh involved.
            meshClusterInfo.assign(naniteMesh.blob.clusterInfos(),
                naniteMesh.blob.clusterInfos() + naniteMesh.blob.clusterCount());
            meshErrorInfo.assign(naniteMesh.blob.errorInfos(),
                naniteMesh.blob.errorInfos() + naniteMesh.blob.errorCount());
        }
        else {
            naniteMesh.buildClusterInfo();
            meshClusterInfo.swap(naniteMesh.clusterInfo);
            meshErrorInfo.swap(naniteMesh.errorInfo);
        }
        clusterIndexOffsets[i] = static_cast<uint32_t>(clusterInfo.size());
        for (auto ci : meshClusterInfo)
        {
            if (i != 0) {
                ci.triangleIndicesStart += indexCounts[i - 1] / 3;
                ci.triangleIndicesEnd += indexCounts[i - 1] / 3;
            }
            clusterInfo.push_back(ci);
        }
        clusterIndexCounts[i] = static_cast<uint32_t>(clusterInfo.size());
        errorInfo.insert(errorInfo.end(), meshErrorInfo.begin(), meshErrorInfo.end());
    }
    ASSERT(clusterInfo.size() == errorInfo.size(), "clusterInfo.size() should be equal to errorInfo.size()");

    for (int i = 0; i < naniteObjects.size(); ++i) {
        auto& naniteObject = naniteObjects[i];
        auto referenceMeshIndex = meshIndexOf(naniteObject);
        sceneIndicesCount += indexCounts[referenceMeshIndex];
        maxClusterNum += clusterIndexCounts[referenceMeshIndex];
    }
}

void NaniteScene::createBVHNodeInfos(vks::VulkanDevice* device, VkQueue transferQueue)
{
    // TODO: Modify this part for multi-mesh!
    for (size_t i = 0; i < naniteMeshes.size(); i++)
    {
        auto & naniteMesh = naniteMeshes[i];
        for (size_t j = 0; j < naniteMesh.sortedClusterIndices.size(); j++)
        {
            sortedClusterIndices.push_back(naniteMesh.sortedClusterIndices[j] + clusterIndexOffsets[i]);
        }
    }
    virtualRootNode = std::make_shared<NaniteBVHNode>();
    virtualRootNode->nodeStatus = VIRTUAL_NODE;
    //virtualRootNode->depth = 0;
    for (int i = 0; i < naniteObjects.size(); ++i) {
        auto& naniteObject = naniteObjects[i];
        naniteObject.reconstructBVH();
        auto referenceMeshIndex = meshIndexOf(naniteObject);
        naniteObject.rootNode->objectIdx = i;
        naniteObject.rootNode->meshIdx = referenceMeshIndex;
        virtualRootNode->children.push_back(naniteObject.rootNode);
    }

    // We should:
    //  Only pack Non-virtual nodes
    //  Only store information that are useful for subsequent traversal
    //      aabb
    //      parent error
    //      children indices(glm::ivec4)
    //  Some meta information about bvh should also be stored (in a uniform buffer)
    //      Node count of each level (At least we should know the node count of level 0 to initiate traversal)


    std::vector<std::shared_ptr<NaniteBVHNode>> flattenedNonVirtualNodes;

    std::queue<std::shared_ptr<NaniteBVHNode>> nodeQueue;
    nodeQueue.push(virtualRootNode);

    uint32_t index = 0;
    // Do a two-pass tree bfs
    //      First pass update cepth & push non-virtual node into vectors
    //      Second pass transform Node to NodeInfo
    while (!nodeQueue.empty())
    {
        auto currNode = nodeQueue.front();
        ASSERT(currNode->nodeStatus != NaniteBVHNodeStatus::INVALID, "Invalid node!");
        nodeQueue.pop();
        if (currNode->nodeStatus != VIRTUAL_NODE) // push all non-virtual nodes into arrays 
        {
            flattenedNonVirtualNodes.push_back(currNode);
            currNode->index = flattenedNonVirtualNodes.size() - 1;
        }
        for (auto child: currNode->children)
        {
            //std::cout << currNode->depth << " " << currNode->objectIdx;
            if (currNode != virtualRootNode) {
                child->objectIdx = currNode->objectIdx;
                child->meshIdx = currNode->meshIdx;
            }
            //child->depth = currNode->depth + 1;
            nodeQueue.push(child);
        }
    }
    
    bvhNodeInfos.resize(flattenedNonVirtualNodes.size());

    for (size_t i = 0; i < flattenedNonVirtualNodes.size(); i++)
    {
        auto& currNode = flattenedNonVirtualNodes[i];

        std::string indent(currNode->depth, '\t');
        //std::cout << indent 
        //    << (currNode->nodeStatus == NaniteBVHNodeStatus::LEAF ? "Leaf " : "Non-leaf ") 
        //    << currNode->depth << " " 
        //    << currNode->index << " " 
        //    << currNode->objectIdx << " "
        //    << std::endl;
        
        //ASSERT(currNode->nodeStatus == VIRTUAL_NODE && currNode->depth >= 0, "A non-virtual node should now be cut out from the bvh tree");
        //ASSERT(currNode->nodeStatus != VIRTUAL_NODE && currNode->depth < 0, "A virtual node should not be cut out from the bvh tree");
        
        auto & nodeInfo = bvhNodeInfos[i];
        nodeInfo.pMinWorld = currNode->pMin;
        nodeInfo.pMaxWorld = currNode->pMax;
        nodeInfo.objectId = currNode->objectIdx;
        nodeInfo.errorWorld.x = currNode->normalizedlodError;
        nodeInfo.errorWorld.y = currNode->parentNormalizedError;
        nodeInfo.errorRP = currNode->parentBoundingSphere;
        nodeInfo.clusterIntervals.x = currNode->start + clusterIndexOffsets[currNode->meshIdx];
        nodeInfo.clusterIntervals.y = currNode->end + clusterIndexOffsets[currNode->meshIdx];
        //std::cout << indent << (currNode->nodeStatus == VIRTUAL_NODE ? "Virtual " : "Non-virtual ")
        //    << " pMin: " << nodeInfo.pMinWorld.x << " " << nodeInfo.pMinWorld.y << " " << nodeInfo.pMinWorld.z
        //    << " pMax: " << nodeInfo.pMaxWorld.x << " " << nodeInfo.pMaxWorld.y << " " << nodeInfo.pMaxWorld.z << std::endl;

        ASSERT(currNode->children.size() <= 4, "Invalid node!");
        for (size_t j = 0; j < currNode->children.size(); ++j)
        {
            auto child = currNode->children[j];
            nodeInfo.childrenNodeIndices[j] = child->index;
        }

        //nodeInfo.clusterIndices = currNode->clusterIndices;
        // TODO: Use memcpy
        //for (size_t i = 0; i < CLUSTER_GROUP_MAX_SIZE; i++)
        //{
        //    if (currNode->clusterIndices[i] >= 0) {
        //        //nodeInfo.clusterIndices[i] = currNode->clusterIndices[i] + clusterIndexOffsets[currNode->objectIdx];
        //        // TODO: Modify this part for multi-mesh!
        //        nodeInfo.clusterIndices[i] = currNode->clusterIndices[i];
        //    }
        //}
        //ASSERT(currNode->nodeStatus == LEAF || nodeInfo.clusterIndices[0] == -1, "A non-leaf node should have no cluster index");
        //std::cout << indent << "object: " << currNode->objectIdx
        //    << " lod: " << currNode->lodLevel
        //    << " depth: " << currNode->depth << std::endl;
        ASSERT(currNode->depth <= depthCounts.size(), "`depth` should never be over depthCounts.size()");
        if (currNode->depth == depthCounts.size())
        {
            depthCounts.push_back(1);
        }
        else
        {
            depthCounts[currNode->depth] += 1;
        }
        if (currNode->nodeStatus == LEAF)
        {
            ASSERT(currNode->clusterIndices.size() <= CLUSTER_GROUP_MAX_SIZE, "this leaf node stores too many cluster indices");
            int validClusterIndicesSize = 0;
            bool isValid = true;
            // Final check
            for (size_t i = 0; i < CLUSTER_GROUP_MAX_SIZE; i++)
            {
                if (currNode->clusterIndices[i] >= 0) {
                    ASSERT(isValid, "Invalid cluster indices"); // This assertion is to make sure that all cluster indices are allocated contiguously
					validClusterIndicesSize += nodeInfo.clusterIntervals[1] - nodeInfo.clusterIntervals[0];
					//std::cout << indent
                    //    << " cluster: " << nodeInfo.clusterIndices[i]
                    //    << std::endl;
                    //ASSERT(clusterIndexSets.find(nodeInfo.clusterIndices[i]) != clusterIndexSets.end(), "Already removed, repeat index!");
                    //clusterIndexSets.erase(nodeInfo.clusterIndices[i]);
				}
                else {
                    isValid = false;
                }
            }
            if (currNode->depth >= depthLeafCounts.size()) {
                depthLeafCounts.resize(currNode->depth + 1);
                depthLeafCounts[currNode->depth] = validClusterIndicesSize;
            }
            else {
                depthLeafCounts[currNode->depth] += validClusterIndicesSize;
            }
            //std::cout << indent << "Curr leaf node clusterIndices size: " << validClusterIndicesSize << std::endl;
        }
        //std::cout << indent << "nodeInfo.clusterIndices[0]: " << nodeInfo.clusterIndices[0] << std::endl;
        //for (size_t i = 0; i < CLUSTER_GROUP_MAX_SIZE; i++)
        //{
        //    std::cout << indent << "clusterIndex: " << currNode->clusterIndices[i] << std::endl;
        //    std::cout << indent << "clusterIndex + offset: " << nodeInfo.clusterIndices[i] << std::endl;
        //}
    }

    //ASSERT(clusterIndexSets.size() == 0, "Some clusters are not covered");
    maxDepthCounts = std::max_element(depthCounts.begin(), depthCounts.end())[0];

    initNodeInfoIndices.resize(depthCounts[0] + 1); // Init node info indices by depth 0 node count
    initNodeInfoIndices[0] = depthCounts[0];
    for (size_t i = 1; i <= depthCounts[0]; i++)
    {
        initNodeInfoIndices[i] = i-1;
    }
}

// Blob path for `createBVHNodeInfos`.
//
// The generic path rebuilds a shared_ptr BVH tree per instance (`reconstructBVH`) only so
// that it can immediately walk it again and write the same numbers into a flat array. When
// the mesh came from a blob every number below can be derived directly, so this version
// walks the baked node arrays with a queue of (object, local node index) pairs. The result
// must be bit-identical to what `createBVHNodeInfos` produces.
void NaniteScene::linkBlobBVHNodeInfos()
{
    ASSERT(allMeshesBaked(), "linkBlobBVHNodeInfos requires every mesh to be blob-loaded");

    // 1. Leaf -> cluster table: the blob stores it with the per-LOD offset only, the scene
    //    adds the per-mesh offset.
    sortedClusterIndices.clear();
    for (size_t i = 0; i < naniteMeshes.size(); i++)
    {
        const auto& blob = naniteMeshes[i].blob;
        for (size_t j = 0; j < blob.sortedClusterIndices.size(); j++)
        {
            sortedClusterIndices.push_back(blob.sortedClusterIndices[j] + clusterIndexOffsets[i]);
        }
    }

    // 2. BFS over the whole scene BVH, skipping the two virtual levels (the scene virtual
    //    root and each instance's virtual root). Because virtual nodes are never emitted,
    //    this yields exactly the order the generic BFS produces.
    struct Entry
    {
        uint32_t objectIdx;
        uint32_t meshIdx;
        uint32_t localIdx;
    };
    std::vector<Entry> queue;
    std::vector<std::vector<int32_t>> sceneIndexOfLocal(naniteObjects.size());
    for (size_t o = 0; o < naniteObjects.size(); ++o)
    {
        NaniteMesh* mesh = naniteObjects[o].referenceMesh;
        ASSERT(mesh && mesh->blobLoaded, "instance refers to a mesh without a baked blob");
        const size_t meshIdx = meshIndexOf(naniteObjects[o]);
        sceneIndexOfLocal[o].assign(mesh->blob.bvhNodes.size(), -1);
        for (uint32_t root : mesh->blob.bvhRoots)
        {
            queue.push_back({ static_cast<uint32_t>(o), static_cast<uint32_t>(meshIdx), root });
        }
    }

    bvhNodeInfos.clear();
    bvhNodeInfos.reserve(queue.size() * 2);
    depthCounts.clear();
    depthLeafCounts.clear();

    for (size_t head = 0; head < queue.size(); ++head)
    {
        const Entry entry = queue[head];
        const NaniteBlob::Asset& blob = naniteObjects[entry.objectIdx].referenceMesh->blob;
        const NaniteBlob::BlobBVHNode& node = blob.bvhNodes[entry.localIdx];
        const glm::mat4& rootTransform = naniteObjects[entry.objectIdx].rootTransform;

        const uint32_t sceneIdx = static_cast<uint32_t>(bvhNodeInfos.size());
        sceneIndexOfLocal[entry.objectIdx][entry.localIdx] = static_cast<int32_t>(sceneIdx);

        auto& nodeInfo = bvhNodeInfos.emplace_back();
        nodeInfo.pMinWorld = glm::vec3(rootTransform * glm::vec4(node.pMin, 1.0f));
        nodeInfo.pMaxWorld = glm::vec3(rootTransform * glm::vec4(node.pMax, 1.0f));
        nodeInfo.objectId = entry.objectIdx;
        nodeInfo.errorWorld = node.errorWorld;
        glm::vec3 parentCenter(node.parentBoundingSphere);
        parentCenter = glm::vec3(rootTransform * glm::vec4(parentCenter, 1.0f));
        nodeInfo.errorRP = glm::vec4(parentCenter, node.parentBoundingSphere.w);
        nodeInfo.clusterIntervals.x = static_cast<int32_t>(node.start + clusterIndexOffsets[entry.meshIdx]);
        nodeInfo.clusterIntervals.y = static_cast<int32_t>(node.end + clusterIndexOffsets[entry.meshIdx]);

        for (size_t j = 0; j < 4; ++j)
        {
            const int32_t child = node.children[static_cast<int>(j)];
            if (child < 0)
            {
                continue; // childrenNodeIndices stays -1, as in the generic path
            }
            // Virtual nodes are never queued, so every queued entry is emitted exactly
            // once and in queue order: a node's queue position *is* its index in
            // `bvhNodeInfos`. That makes the child's slot known the moment it is queued,
            // which is what the generic path only learns when it pops the child.
            int32_t& childSlot = sceneIndexOfLocal[entry.objectIdx][static_cast<size_t>(child)];
            if (childSlot < 0)
            {
                childSlot = static_cast<int32_t>(queue.size());
                queue.push_back({ entry.objectIdx, entry.meshIdx, static_cast<uint32_t>(child) });
            }
            nodeInfo.childrenNodeIndices[static_cast<int>(j)] = childSlot;
        }

        ASSERT(static_cast<int>(node.depth) <= static_cast<int>(depthCounts.size()), "`depth` should never be over depthCounts.size()");
        if (node.depth == depthCounts.size())
        {
            depthCounts.push_back(1);
        }
        else
        {
            depthCounts[node.depth] += 1;
        }

        if (node.nodeStatus == LEAF)
        {
            // Stats only. The generic path sums the node's cluster interval once per
            // valid cluster index; there are exactly `end - start` of those, so the
            // accumulated amount is the interval length squared.
            const uint32_t clustersInNode = node.end - node.start;
            const uint32_t validClusterIndicesSize = clustersInNode * clustersInNode;
            if (node.depth >= depthLeafCounts.size())
            {
                depthLeafCounts.resize(node.depth + 1);
                depthLeafCounts[node.depth] = validClusterIndicesSize;
            }
            else
            {
                depthLeafCounts[node.depth] += validClusterIndicesSize;
            }
        }
    }

    // Every baked node except the mesh's own virtual root (which is never emitted, its
    // children are the `bvhRoots` seeds above) has to be reachable.
    for (size_t o = 0; o < naniteObjects.size(); ++o)
    {
        const auto& nodes = naniteObjects[o].referenceMesh->blob.bvhNodes;
        for (size_t i = 0; i < nodes.size(); ++i)
        {
            if (nodes[i].nodeStatus == VIRTUAL_NODE)
            {
                continue;
            }
            ASSERT(sceneIndexOfLocal[o][i] >= 0, "the blob BVH is not fully connected - a baked node is unreachable");
        }
    }

    maxDepthCounts = std::max_element(depthCounts.begin(), depthCounts.end())[0];
    initNodeInfoIndices.resize(depthCounts[0] + 1);
    initNodeInfoIndices[0] = depthCounts[0];
    for (size_t i = 1; i <= depthCounts[0]; i++)
    {
        initNodeInfoIndices[i] = static_cast<uint32_t>(i - 1);
    }
}
