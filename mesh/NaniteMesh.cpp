#include "NaniteMesh.h"

#include <cstring>

void NaniteMesh::loadvkglTFModel(const vkglTF::Model& model)
{
	vkglTFModel = &model;
	for (auto& node : vkglTFModel->linearNodes)
	{
		// TODO: Only support naniting one mesh within a model
		if (node->mesh) {
			vkglTFMesh = node->mesh;
			modelMatrix = node->getMatrix();
			break;
		}
	}
}

void NaniteMesh::vkglTFPrimitiveToOpenMesh(MyMesh& mymesh, const vkglTF::Primitive& prim)
{
	int vertStart = prim.firstVertex;
	int vertEnd = prim.firstVertex + prim.vertexCount;
	std::vector<MyMesh::VertexHandle> vhandles;
	for (int i = vertStart; i != vertEnd; i++)
	{
		auto& vert = vkglTFModel->vertexBuffer[i];
		auto vhandle = mymesh.add_vertex(MyMesh::Point(vert.pos.x, vert.pos.y, vert.pos.z));
		mymesh.set_normal(vhandle, MyMesh::Normal(vert.normal.x, vert.normal.y, vert.normal.z));
		mymesh.set_texcoord2D(vhandle, MyMesh::TexCoord2D(vert.uv.x, vert.uv.y));
		vhandles.emplace_back(vhandle);
	}
	int indStart = prim.firstIndex;
	int indEnd = prim.firstIndex + prim.indexCount;
	for (int i = indStart; i != indEnd; i += 3)
	{
		int i0 = vkglTFModel->indexBuffer[i] - vertStart, i1 = vkglTFModel->indexBuffer[i + 1] - vertStart, i2 = vkglTFModel->indexBuffer[i + 2] - vertStart;
		std::vector<MyMesh::VertexHandle> face_vhandles;
		face_vhandles.clear();
		face_vhandles.emplace_back(vhandles[i0]);
		face_vhandles.emplace_back(vhandles[i1]);
		face_vhandles.emplace_back(vhandles[i2]);
		mymesh.add_face(face_vhandles);
	}
}


void NaniteMesh::vkglTFMeshToOpenMesh(MyMesh& mymesh, const vkglTF::Mesh& mesh) {
	for (auto& prim : mesh.primitives)
	{
		vkglTFPrimitiveToOpenMesh(mymesh, *prim);

		mymesh.request_face_status();
		mymesh.request_edge_status();
		mymesh.request_vertex_status();
	}
}

void NaniteMesh::flattenDAG()
{
	for (int i = meshes.size()-1; i >= 0; i--)
	{
		auto& mesh = meshes[i];
		for (size_t clusterIdx = 0; clusterIdx < mesh.clusters.size(); clusterIdx++)
		{
			const auto & cluster = mesh.clusters[clusterIdx];
			ASSERT(i == cluster.lodLevel, "lod level not match");
			//std::cout << "Cluster " << clusterIdx 
			//	<< " lod " << i 
			//	<< " parentError " << cluster.parentError
			//	<< " lodError " << cluster.lodError
			//	<< std::endl;
			float normalizedParentError = (i == meshes.size() - 1) ? cluster.parentNormalizedError / cluster.parentSurfaceArea : FLT_MAX;
			float normalizedLodError = cluster.lodError / cluster.surfaceArea;
			ASSERT(cluster.parentSurfaceArea > 0 || i == meshes.size()-1, "parentSurfaceArea should be positive");
			ASSERT(cluster.surfaceArea > 0, "surfaceArea should be positive");
			flattenedClusterNodes.emplace_back(ClusterNode({ 
				normalizedParentError,
				normalizedLodError,
				cluster.boundingSphereCenter, 
				cluster.boundingSphereRadius 
			}));
		}
	}
}

void NaniteMesh::flattenBVH()
{
	virtualBVHRootNode = std::make_shared<NaniteBVHNode>();
	virtualBVHRootNode->nodeStatus = NaniteBVHNodeStatus::VIRTUAL_NODE;
	for (const auto & mesh: meshes)
	{
		virtualBVHRootNode->children.push_back(mesh.rootBVHNode);
	}

	std::queue<std::shared_ptr<NaniteBVHNode>> nodeQueue;
	nodeQueue.push(virtualBVHRootNode);

	uint32_t index = 0;

	std::vector<uint32_t> depthCounts;
	uint32_t maxLevels = 0;
	// Do a two-pass tree bfs
	//      First pass update flattened index
	//      Second pass update children index
	while (!nodeQueue.empty())
	{
		auto currNode = nodeQueue.front();
		currNode->index = index;
		std::string indent(currNode->depth, '\t');
		ASSERT(currNode->nodeStatus != NaniteBVHNodeStatus::INVALID, "Invalid node!");

		//std::cout << indent << (currNode->nodeStatus == NaniteBVHNodeStatus::LEAF ? "Leaf " : "Non-leaf ") << currNode->depth << " " << currNode->index << std::endl;
		//ASSERT(currNode->depth <= depthCounts.size(), "currNode->level should never be greater than size of depthCounts, check traversal implementation");
		if (currNode->depth == depthCounts.size())
		{
			depthCounts.push_back(1);
		}
		else
		{
			depthCounts[currNode->depth] += 1;
		}
		index++;
		nodeQueue.pop();
		std::cout << indent << "currNode->index: " << currNode->index 
			<< std::endl << indent << "currNode->parentBoundingSphereCenter: " << currNode->parentBoundingSphere.x << " " << currNode->parentBoundingSphere.y << " " << currNode->parentBoundingSphere.z
			<< std::endl << indent << "currNode->parentBoundingSphereRadius: " << currNode->parentBoundingSphere.w
			<< std::endl;
		if (currNode->nodeStatus == NaniteBVHNodeStatus::LEAF)
		{
			//std::cout << "Leaf node: " <<
			//    "pMin: " << currNode->pMin.x << " " << currNode->pMin.y << " " << currNode->pMin.z <<
			//    "pMax: " << currNode->pMax.x << " " << currNode->pMax.y << " " << currNode->pMax.z <<
			//    std::endl;
		}
		else
		{
			//std::cout << "Non-leaf node: "
			//    "pMin: " << currNode->pMin.x << " " << currNode->pMin.y << " " << currNode->pMin.z <<
			//    "pMax: " << currNode->pMax.x << " " << currNode->pMax.y << " " << currNode->pMax.z <<
			//    std::endl;
			for (auto child : currNode->children)
			{
				//std::cout << indent << child->depth << std::endl;
				nodeQueue.push(child);
			}
		}
	}
	flattenedBVHNodeInfos.resize(index);  // `index` now is the size of nodes
	//for (size_t i = 0; i < depthCounts.size(); i++)
	//{
	//    std::cout << "i " << i << " depthCounts[i]: " << depthCounts[i] << std::endl;
	//}

	uint32_t totalClusterNum = 0;
	for (size_t i = 0; i < meshes.size(); i++)
	{
		totalClusterNum += meshes[i].clusterNum;
	}
	std::unordered_set<uint32_t> clusterIndexSet;
	for (size_t i = 0; i < totalClusterNum; i++)
	{
		clusterIndexSet.insert(i);
	}

	nodeQueue.push(virtualBVHRootNode);
	while (!nodeQueue.empty())
	{

		auto currNode = nodeQueue.front();
		NaniteBVHNodeInfo nodeInfo;
		//std::cout << currNode->children.size() << std::endl;
		ASSERT(currNode->nodeStatus == VIRTUAL_NODE || currNode->children.size() <= 4, "size of non-virtual nodes' children should never be over 4");
		nodeInfo.children.resize(currNode->children.size());
		for (int i = 0; i < currNode->children.size(); ++i)
		{
			nodeInfo.children[i] = currNode->children[i]->index;
		}
		nodeInfo.pMax = currNode->pMax;
		nodeInfo.pMin = currNode->pMin;
		nodeInfo.parentNormalizedError = currNode->parentNormalizedError;
		nodeInfo.normalizedlodError = currNode->normalizedlodError;
		nodeInfo.parentBoundingSphere = currNode->parentBoundingSphere;
		nodeInfo.nodeStatus = currNode->nodeStatus;
		nodeInfo.depth = currNode->depth;
		nodeInfo.index = currNode->index;
		nodeInfo.clusterIndices = currNode->clusterIndices;
		nodeInfo.lodLevel = currNode->lodLevel;
		ASSERT(flattenedBVHNodeInfos[currNode->index].nodeStatus == INVALID, "Repeated index!");
		ASSERT(currNode->index < flattenedBVHNodeInfos.size(), "index over flattenedBVHNodeInfos.size()");
		nodeQueue.pop();
		if (currNode->nodeStatus == NaniteBVHNodeStatus::LEAF)
		{
			ASSERT(currNode->clusterIndices.size() <= CLUSTER_GROUP_MAX_SIZE, "cluster group size over threshold!");
			int validClusterNum = 0;
			for (size_t i = 0; i < CLUSTER_GROUP_MAX_SIZE; i++)
			{
				auto clusterIndex = currNode->clusterIndices[i];
				if (clusterIndex >= 0) {
					validClusterNum++;
				}
			}
			nodeInfo.start = sortedClusterIndices.size();
			for (size_t i = 0; i < validClusterNum; i++)
			{
				sortedClusterIndices.push_back(currNode->clusterIndices[i] + clusterIndexOffset[currNode->lodLevel]);
				ASSERT(clusterIndexSet.find(currNode->clusterIndices[i] + clusterIndexOffset[currNode->lodLevel]) != clusterIndexSet.end(), "Repeated cluster index!");
				clusterIndexSet.erase(currNode->clusterIndices[i] + clusterIndexOffset[currNode->lodLevel]);
			}
			nodeInfo.end = sortedClusterIndices.size();
			std::cout << nodeInfo.start << " " << nodeInfo.end << std::endl;
			//for (size_t i = 0; i < CLUSTER_GROUP_MAX_SIZE; i++)
			//{
			//	std::cout << std::string(currNode->depth, '\t') << "nodeInfo: " << nodeInfo.clusterIndices[i] << std::endl;
			//	std::cout << std::string(currNode->depth, '\t') << "currNode: " << currNode->clusterIndices[i] << std::endl;
			//}
			//std::cout << "Leaf node: " <<
			//	"pMin: " << currNode->pMin.x << " " << currNode->pMin.y << " " << currNode->pMin.z <<
			//	"pMax: " << currNode->pMax.x << " " << currNode->pMax.y << " " << currNode->pMax.z <<
			//	std::endl;
		}
		else
		{
			//std::cout << "Non-leaf node: "
			//	"pMin: " << currNode->pMin.x << " " << currNode->pMin.y << " " << currNode->pMin.z <<
			//	"pMax: " << currNode->pMax.x << " " << currNode->pMax.y << " " << currNode->pMax.z <<
			//	std::endl;
			for (auto child : currNode->children)
			{
				nodeQueue.push(child);
			}
		}
		flattenedBVHNodeInfos[currNode->index] = nodeInfo;
	}

	ASSERT(clusterIndexSet.size() == 0, "Some cluster indices are not assigned to any node!");
}

void NaniteMesh::generateNaniteInfo() {
	MyMesh mymesh;
	vkglTFMeshToOpenMesh(mymesh, *vkglTFMesh);
	int clusterGroupNum = -1;
	int target = 6;
	int currFaceNum = -1;
	/*if (!OpenMesh::IO::read_mesh(mymesh, "D:\\AndrewChen\\CIS565\\Vulcanite\\assets\\models\\bunny.obj")) {
		ASSERT(0, "failed to load mesh");
	}*/
	// Add a customized property to store clusterGroupIndex of last level of detail
	mymesh.add_property(clusterGroupIndexPropHandle);
	do
	{
		// For each lod mesh
		Mesh meshLOD;
		meshLOD.mesh = mymesh;
		meshLOD.lodLevel = lodNums;
		meshLOD.clusterGroupIndexPropHandle = clusterGroupIndexPropHandle;
		if (clusterGroupNum > 0) {
			meshLOD.oldClusterGroups.resize(clusterGroupNum);
			meshLOD.assignTriangleClusterGroup(meshes.back()); 
		}
		else {
			meshLOD.buildTriangleGraph();
			meshLOD.generateCluster();
		}
		if (meshes.size() > 0) {
			auto& lastMeshLOD = meshes[meshes.size() - 1];
			// Maintain DAG

		}
		// Generate cluster group by partitioning cluster graph
		meshLOD.buildClusterGraph();
		meshLOD.colorClusterGraph(); // Cluster graph is needed to assign adjacent cluster different colors
		meshLOD.generateClusterGroup();
		currFaceNum = meshLOD.mesh.n_faces();
		clusterGroupNum = meshLOD.clusterGroupNum;

		mymesh = meshLOD.mesh;
		if (clusterGroupNum > 1) 
		{
			meshLOD.simplifyMesh(mymesh); 
			// Save LOD mesh for debugging
			//{
			//	std::string output_filename = "meshLOD_" + std::to_string(lodNums) + ".obj";
			//
			//	// Export the mesh to the specified file
			//	if (!OpenMesh::IO::write_mesh(mymesh, output_filename)) {
			//		std::cerr << "Error exporting mesh to " << output_filename << std::endl;
			//	}
			//}
		}
		meshes.emplace_back(meshLOD);
		std::cout << "LOD " << lodNums++ << " generated" << std::endl;

	} 
	//while (clusterGroupNum != 1 &&
	//  mymesh.n_faces() != currFaceNum // Decimation no longer decrease faces
	//); 
	while (--target); // Only do one time for testing
	// Linearize DAG
	
	//flattenDAG();
	clusterIndexOffset.resize(meshes.size(), 0);
	for (size_t i = 0; i < meshes.size(); i++)
	{
		if (i != 0) {
			clusterIndexOffset[i] = clusterIndexOffset[i - 1] + meshes[i - 1].clusterNum;
		}
		meshes[i].createBVH();
	}
	// Linearize BVH
	flattenBVH();
	
	// Save mesh for debugging
	//{
	//	std::string output_filename = "output.obj";
	//
	//	// Export the mesh to the specified file
	//	if (!OpenMesh::IO::write_mesh(mymesh, output_filename)) {
	//		std::cerr << "Error exporting mesh to " << output_filename << std::endl;
	//	}
	//}
}

bool NaniteMesh::serialize(const std::string& filepath)
{
	std::filesystem::path directoryPath(filepath);

	// Caching is an optimization, not a requirement: a model picked from a read-only or
	// network folder has to keep working, it just gets rebuilt on the next run.
	std::error_code ec;
	std::filesystem::create_directories(directoryPath, ec);
	if (ec) {
		std::cerr << "[cache] cannot create " << filepath << ": " << ec.message()
			<< " - continuing without a cache" << std::endl;
		return false;
	}

	for (size_t i = 0; i < meshes.size(); i++)
	{
		auto& mesh = meshes[i];
		std::string output_filename = std::string(filepath) + "LOD_" + std::to_string(i) + ".obj";
		// Export the mesh to the specified file
		if (!OpenMesh::IO::write_mesh(mesh.mesh, output_filename, OpenMesh::IO::Options::VertexNormal  | OpenMesh::IO::Options::VertexTexCoord)) {
			std::cerr << "Error exporting mesh to " << output_filename << std::endl;
		}
	}

	json result;
	for (size_t i = 0; i < meshes.size(); i++)
	{
		result["mesh"][i] = meshes[i].toJson();
	}
	for (size_t i = 0; i < flattenedBVHNodeInfos.size(); i++)
	{
		result["flattenedBVHNodeInfos"][i] = flattenedBVHNodeInfos[i].toJson();
	}
	result["flattenedBVHNodeCounts"] = flattenedBVHNodeInfos.size();
	result[cache_time_key] = std::time(nullptr);
	result["lodNums"] = lodNums;
	result["sortedClusterIndices"] = sortedClusterIndices;

	// Save the JSON data to a file
	std::ofstream file(std::string(filepath) + "nanite_info.json");
	if (!file.is_open()) {
		std::cerr << "[cache] cannot write " << filepath << "nanite_info.json"
			<< " - continuing without a cache" << std::endl;
		return false;
	}
	file << result.dump(2); // Pretty-print with an indentation of 2 spaces
	file.close();
	if (!file.good()) {
		std::cerr << "[cache] write error for " << filepath << "nanite_info.json"
			<< " - continuing without a cache" << std::endl;
		return false;
	}
	return true;
}

void NaniteMesh::deserialize(const std::string & filepath)
{
	std::ifstream inputFile(std::string(filepath) + "nanite_info.json");

	ASSERT(inputFile.is_open(), "Error opening file for deserialization");
	json loadedJson;
	inputFile >> loadedJson;
		
	lodNums = loadedJson["lodNums"].get<uint32_t>();
	meshes.resize(lodNums);
	for (int i = 0; i < lodNums; ++i) {
		auto& meshLOD = meshes[i];
		meshLOD.fromJson(loadedJson["mesh"][i]);

		float percentage = static_cast<float>(i+1) / lodNums * 100.0;
		std::cout << "\r";
		std::cout << "[Loading] Mesh Info: " << std::fixed << std::setw(6) << std::setprecision(2) << percentage << "%";
		std::cout.flush();
	}
	std::cout << std::endl;

	int flattenedBVHNodeCounts = loadedJson["flattenedBVHNodeCounts"].get<uint32_t>();
	flattenedBVHNodeInfos.resize(flattenedBVHNodeCounts, NaniteBVHNodeInfo());
	for (int i = 0; i < flattenedBVHNodeCounts; ++i) {
		auto& nodeInfo = flattenedBVHNodeInfos[i];
		nodeInfo.fromJson(loadedJson["flattenedBVHNodeInfos"][i]);
		std::string indent(nodeInfo.depth, '\t');
		//if (nodeInfo.nodeStatus == LEAF) {
		//	std::cout << "Index: " << nodeInfo.index << std::endl;
		//	for (size_t i = 0; i < nodeInfo.clusterIndices.size(); i++)
		//	{
		//		std::cout << '\t' << nodeInfo.clusterIndices[i] << std::endl;
		//	}
		//}
		float percentage = static_cast<float>(i + 1) / flattenedBVHNodeCounts * 100.0;
		std::cout << "\r";
		std::cout << "[Loading] BVH Info: " << std::fixed << std::setw(6) << std::setprecision(2) << percentage << "%";
		std::cout.flush();
	}
	std::cout << std::endl;

	for (size_t i = 0; i < loadedJson["sortedClusterIndices"].size(); i++)
	{
		sortedClusterIndices.push_back(loadedJson["sortedClusterIndices"][i].get<uint32_t>());
	}

	for (size_t i = 0; i < lodNums; i++)
	{
		std::string output_filename = std::string(filepath) + "LOD_" + std::to_string(i) + ".obj";
		meshes[i].mesh.request_vertex_normals();
		meshes[i].mesh.request_vertex_texcoords2D();
		OpenMesh::IO::Options opt = OpenMesh::IO::Options::VertexNormal | OpenMesh::IO::Options::VertexTexCoord;
		if (!OpenMesh::IO::read_mesh(meshes[i].mesh, output_filename, opt)) {
			ASSERT(0, "failed to load mesh");
		}
		ASSERT(meshes[i].mesh.has_vertex_normals(), "mesh has no normals");
		meshes[i].lodLevel = i;

		std::cout << "\r";
		float percentage = static_cast<float>(i+1) / lodNums * 100.0;
		std::cout << "[Loading] Mesh LOD: " << std::fixed << std::setw(6) << std::setprecision(2) << percentage << "%";
		std::cout.flush();
	}
	std::cout << std::endl;
}

std::string NaniteMesh::cachePathFor(const std::string& filepath)
{
	const size_t dot = filepath.find_last_of(".");
	if (dot == std::string::npos) {
		ASSERT(0, "Invalid file path, no ext");
		return "";
	}
	// `<model>_naniteCache` next to the source mesh. The trailing separator comes from
	// std::filesystem so the result stays valid on POSIX hosts instead of hardcoding `\\`.
	std::filesystem::path cachePath(filepath.substr(0, dot) + "_naniteCache");
	cachePath += std::filesystem::path::preferred_separator;
	return cachePath.string();
}

std::string NaniteMesh::blobPathFor(const std::string& filepath)
{
	return cachePathFor(filepath) + "nanite.blob";
}

void NaniteMesh::bakeBlob(const std::string& filepath)
{
	NaniteBlob::Asset asset;
	asset.sourcePath = filepath;
	asset.lodNums = lodNums;
	asset.vertexStride = static_cast<uint32_t>(sizeof(vkglTF::Vertex));

	// --- vertices / indices: exactly the layout `Instance::initBufferForNaniteLODs` produces ---
	size_t totalVerts = 0, totalIndices = 0;
	for (size_t i = 0; i < meshes.size(); i++)
	{
		totalVerts += meshes[i].uniqueVertexBuffer.size();
		totalIndices += meshes[i].triangleVertexIndicesSortedByClusterIdx.size();
	}
	asset.vertices.assign(totalVerts * sizeof(vkglTF::Vertex), 0);
	asset.indices.resize(totalIndices);
	size_t vertexCursor = 0, indexCursor = 0, runningVerts = 0;
	for (size_t i = 0; i < meshes.size(); i++)
	{
		if (!meshes[i].uniqueVertexBuffer.empty()) {
			std::memcpy(asset.vertices.data() + vertexCursor * sizeof(vkglTF::Vertex),
				meshes[i].uniqueVertexBuffer.data(),
				meshes[i].uniqueVertexBuffer.size() * sizeof(vkglTF::Vertex));
		}
		vertexCursor += meshes[i].uniqueVertexBuffer.size();
		for (size_t j = 0; j < meshes[i].triangleVertexIndicesSortedByClusterIdx.size(); j++)
		{
			asset.indices[indexCursor++] =
				static_cast<uint32_t>(meshes[i].triangleVertexIndicesSortedByClusterIdx[j])
				+ static_cast<uint32_t>(runningVerts);
		}
		runningVerts += meshes[i].uniqueVertexBuffer.size();
	}
	ASSERT(vertexCursor == totalVerts && indexCursor == totalIndices, "blob: vertex/index bake mismatch");

	// --- cluster / error info: mesh-relative, `objectIdx` intentionally left at 0 ---
	buildClusterInfo();
	asset.clusterInfo.resize(clusterInfo.size() * sizeof(ClusterInfo));
	if (!clusterInfo.empty()) {
		std::memcpy(asset.clusterInfo.data(), clusterInfo.data(), asset.clusterInfo.size());
	}
	asset.errorInfo.resize(errorInfo.size() * sizeof(ErrorInfo));
	if (!errorInfo.empty()) {
		std::memcpy(asset.errorInfo.data(), errorInfo.data(), asset.errorInfo.size());
	}

	// --- BVH: keep the builder's local child indices, the scene re-links them later ---
	asset.bvhNodes.resize(flattenedBVHNodeInfos.size());
	for (size_t i = 0; i < flattenedBVHNodeInfos.size(); i++)
	{
		const auto& src = flattenedBVHNodeInfos[i];
		auto& dst = asset.bvhNodes[i];
		dst.pMin = src.pMin;
		dst.pMax = src.pMax;
		dst.parentBoundingSphere = src.parentBoundingSphere;
		dst.children = glm::ivec4(-1);
		// Only the virtual root may have more than 4 children; its children are stored
		// in SECTION_BVH_ROOTS instead.
		for (size_t j = 0; j < src.children.size() && j < 4; j++) {
			dst.children[static_cast<int>(j)] = src.children[j];
		}
		dst.errorWorld = glm::vec2(static_cast<float>(src.normalizedlodError), static_cast<float>(src.parentNormalizedError));
		dst.start = static_cast<uint32_t>(src.start);
		dst.end = static_cast<uint32_t>(src.end);
		dst.depth = static_cast<int32_t>(src.depth);
		dst.nodeStatus = static_cast<int32_t>(src.nodeStatus);
		dst.lodLevel = src.lodLevel;
		dst.index = src.index;
	}
	ASSERT(!flattenedBVHNodeInfos.empty(), "blob: BVH was never flattened");
	for (size_t i = 0; i < flattenedBVHNodeInfos[0].children.size(); i++) {
		asset.bvhRoots.push_back(static_cast<uint32_t>(flattenedBVHNodeInfos[0].children[i]));
	}

	// --- leaf -> cluster table, LOD-offset only (the scene adds the mesh offset) ---
	asset.sortedClusterIndices = sortedClusterIndices;

	asset.lodClusterNums.resize(meshes.size());
	for (size_t i = 0; i < meshes.size(); i++) {
		asset.lodClusterNums[i] = static_cast<uint32_t>(meshes[i].clusterNum);
	}

	std::string error;
	if (!NaniteBlob::write(blobPathFor(filepath), asset, NaniteBlob::fnv1aFile(filepath), error)) {
		std::cerr << "[blob] failed to bake " << blobPathFor(filepath) << ": " << error << std::endl;
		return;
	}
	std::cout << "[blob] baked " << blobPathFor(filepath)
		<< " (" << asset.vertexCount() << " verts, " << asset.indices.size() << " indices, "
		<< asset.clusterCount() << " clusters, " << asset.bvhNodes.size() << " bvh nodes)" << std::endl;
}

bool NaniteMesh::loadBlob(const std::string& filepath, bool strictSourceHash)
{
	NaniteBlob::Asset asset;
	asset.sourcePath = filepath;
	std::string error;
	if (!NaniteBlob::read(blobPathFor(filepath), asset, error, strictSourceHash)) {
		std::cerr << "[blob] " << error << std::endl;
		return false;
	}
	blob = std::move(asset);
	lodNums = blob.lodNums;
	blobLoaded = true;
	std::cout << "[blob] loaded " << blobPathFor(filepath)
		<< " (" << blob.vertexCount() << " verts, " << blob.indices.size() << " indices, "
		<< blob.clusterCount() << " clusters, " << blob.bvhNodes.size() << " bvh nodes)" << std::endl;
	return true;
}

void NaniteMesh::initNaniteInfo(const std::string & filepath, bool useCache, BlobMode blobMode) {
	bool hasInitialized = false;
	std::string cachePath = cachePathFor(filepath);

	// 1. The binary blob is the fast path: it holds the final GPU payloads, so the
	//    viewer never touches OpenMesh, METIS or the JSON description of the DAG.
	const bool mayReadBlob = blobMode == BlobMode::Auto || blobMode == BlobMode::Require;
	if (mayReadBlob && (blobMode == BlobMode::Require || useCache)) {
		if (NaniteBlob::exists(blobPathFor(filepath))) {
			if (loadBlob(filepath, blobMode == BlobMode::Require)) {
				hasInitialized = true;
			}
			else if (blobMode == BlobMode::Require) {
				ASSERT(0, "--nanite-blob was requested but the blob cache is unusable");
			}
			else {
				std::cerr << "Blob cache unusable, falling back" << std::endl;
			}
		}
		else if (blobMode == BlobMode::Require) {
			ASSERT(0, "--nanite-blob was requested but no blob cache exists - bake one by running once without it");
		}
	}

	// 2. The legacy JSON cache still short-circuits the (expensive) build.
	//    `BlobMode::Rebuild` is `--nanite-rebuild`: start from the glTF and ignore both caches.
	if (!hasInitialized && blobMode == BlobMode::Auto && useCache) {
		std::ifstream inputFile(cachePath + "nanite_info.json");
		if (inputFile.is_open()) {
			deserialize(cachePath);
			hasInitialized = true;
		}
		else {
			std::cerr << "No cache, need to initialize from now" << std::endl;
		}
	}

	if (!hasInitialized) {
		std::cerr << "Start building..." << std::endl;
		generateNaniteInfo();
		const bool cached = serialize(cachePath);
		if (cached) {
			std::cout << cachePath << "nanite_info.json" << " generated" << std::endl;
		}

		// 3. Bake the binary cache for the next run. The current run keeps using the
		//    freshly built in-memory state, so the two paths stay cross-checked.
		if (cached && blobMode != BlobMode::JsonOnly) {
			for (size_t i = 0; i < meshes.size(); i++) {
				meshes[i].initUniqueVertexBuffer();
			}
			bakeBlob(filepath);
		}
		//checkDeserializationResult(cachePath);
	}
}

void NaniteMesh::checkDeserializationResult(const std::string& filepath)
{
	std::ifstream inputFile(std::string(filepath) + "nanite_info.json");

	ASSERT(inputFile.is_open(), "Error opening file for deserialization");
	json loadedJson;
	inputFile >> loadedJson;

	lodNums = loadedJson["lodNums"].get<uint32_t>();
	debugMeshes.resize(lodNums);
	for (int i = 0; i < lodNums; ++i) {
		auto& meshLOD = debugMeshes[i];
		meshLOD.fromJson(loadedJson["mesh"][i]);
	}

	for (size_t i = 0; i < lodNums; i++)
	{
		std::string output_filename = std::string(filepath) + "LOD_" + std::to_string(i) + ".obj";
		debugMeshes[i].mesh.request_vertex_normals();
		OpenMesh::IO::Options opt = OpenMesh::IO::Options::VertexNormal;
		if (!OpenMesh::IO::read_mesh(debugMeshes[i].mesh, output_filename, opt)) {
			ASSERT(0, "failed to load mesh");
		}
		ASSERT(debugMeshes[i].mesh.has_vertex_normals(), "mesh has no normals");
		debugMeshes[i].lodLevel = i;
	}

	for (size_t i = 0; i < lodNums; i++)
	{
		auto& mesh = meshes[i];
		auto& debugMesh = debugMeshes[i];
		TEST(mesh.clusters.size() == debugMesh.clusters.size(), "cluster size match");
		for (size_t clusterIdx = 0; clusterIdx < mesh.clusters.size(); clusterIdx++)
		{
			const auto& cluster = mesh.clusters[clusterIdx];
			const auto& debugCluster = debugMesh.clusters[clusterIdx];
			TEST(cluster.parentNormalizedError == debugCluster.parentNormalizedError, "parentNormalizedError match");
			TEST(cluster.lodError == debugCluster.lodError, "lodError match");
			TEST(cluster.boundingSphereCenter == debugCluster.boundingSphereCenter, "boundingSphereCenter match");
			TEST(cluster.boundingSphereRadius == debugCluster.boundingSphereRadius, "boundingSphereRadius match");
		}
		TEST(mesh.mesh.n_faces() == debugMesh.mesh.n_faces(), "face size match");
		TEST(mesh.mesh.n_vertices() == debugMesh.mesh.n_vertices(), "vertex size match");
		for (const auto & vhandle: mesh.mesh.vertices())
		{
			auto& debugVhandle = debugMesh.mesh.vertex_handle(vhandle.idx());
			//std::cout << "mesh normal: " << mesh.mesh.normal(vhandle)[0] << " " << mesh.mesh.normal(vhandle)[1] << " " << mesh.mesh.normal(vhandle)[2] << std::endl;
			//std::cout << "debug normal: " << debugMesh.mesh.normal(debugVhandle)[0] << " " << debugMesh.mesh.normal(debugVhandle)[1] << " " << debugMesh.mesh.normal(debugVhandle)[2] << std::endl;
			TEST((mesh.mesh.point(vhandle) - debugMesh.mesh.point(debugVhandle)).length() < 1e-5f, "vertex position match");
			TEST((mesh.mesh.normal(vhandle) - debugMesh.mesh.normal(debugVhandle)).length() < 1e-5f, "vertex normal match");
			//ASSERT((mesh.mesh.texcoord2D(vhandle) - debugMesh.mesh.texcoord2D(debugVhandle)).length() < 1e-5f, "vertex texcoord not match");
		}

	}
}

void NaniteMesh::buildClusterInfo()
{
	// Init Clusters
	size_t totalClusterNum = 0;
	for (int i = 0; i < meshes.size(); i++)
	{
		totalClusterNum += meshes[i].clusterNum;
#ifdef DEBUG_LOD_START
		break;
#endif // DEBUG_LOD_START
	}
	clusterInfo.clear();
	errorInfo.clear();
	clusterInfo.resize(totalClusterNum);
	errorInfo.resize(totalClusterNum);
	size_t currClusterNum = 0, currTriangleNum = 0;
	for (int i = 0; i < meshes.size(); i++)
	{
		auto& mesh = meshes[i].mesh;
		for (MyMesh::FaceIter face_it = mesh.faces_begin(); face_it != mesh.faces_end(); ++face_it) {
			MyMesh::FaceHandle fh = *face_it;
			auto clusterIdx = meshes[i].triangleClusterIndex[fh.idx()] + currClusterNum;
			auto& clusterI = clusterInfo[clusterIdx];

			glm::vec3 pMinWorld, pMaxWorld;
			glm::vec3 p0, p1, p2;
			MyMesh::FaceVertexIter fv_it = mesh.fv_iter(fh);

			// Get the positions of the three vertices
			auto point0 = mesh.point(*fv_it);
			++fv_it;
			auto point1 = mesh.point(*fv_it);
			++fv_it;
			auto point2 = mesh.point(*fv_it);

			p0[0] = point0[0];
			p0[1] = point0[1];
			p0[2] = point0[2];

			p1[0] = point1[0];
			p1[1] = point1[1];
			p1[2] = point1[2];

			p2[0] = point2[0];
			p2[1] = point2[1];
			p2[2] = point2[2];

			p0 = glm::vec3(glm::vec4(p0, 1.0f));
			p1 = glm::vec3(glm::vec4(p1, 1.0f));
			p2 = glm::vec3(glm::vec4(p2, 1.0f));

			getTriangleAABB(p0, p1, p2, pMinWorld, pMaxWorld);

			clusterI.mergeAABB(pMinWorld, pMaxWorld);
		}


		uint32_t currClusterIdx = -1;
		for (size_t j = 0; j < meshes[i].triangleIndicesSortedByClusterIdx.size(); j++)
		{
			auto currTriangleIndex = meshes[i].triangleIndicesSortedByClusterIdx[j];
			if (meshes[i].triangleClusterIndex[currTriangleIndex] != currClusterIdx)
			{
				if (currClusterIdx != -1) {
					//std::cout << "Cluster " << currClusterIdx << " end at " << j << std::endl;
					clusterInfo[currClusterIdx + currClusterNum].triangleIndicesEnd = j + currTriangleNum;
				}
				currClusterIdx = meshes[i].triangleClusterIndex[currTriangleIndex];
				clusterInfo[currClusterIdx + currClusterNum].triangleIndicesStart = j + currTriangleNum;
				//std::cout << "Cluster " << currClusterIdx << " start at " << j << std::endl;
			}
		}
		clusterInfo[currClusterIdx + currClusterNum].triangleIndicesEnd = meshes[i].triangleIndicesSortedByClusterIdx.size() + currTriangleNum;

		for (size_t j = 0; j < meshes[i].clusters.size(); j++)
		{
			auto& cluster = meshes[i].clusters[j];
			float parentError = i == meshes.size() - 1 ? 1e5 : cluster.parentNormalizedError;
			ASSERT(parentError > cluster.normalizedlodError, "Parent error is not greater than children's");
			errorInfo[j + currClusterNum].errorWorld = glm::vec2(cluster.normalizedlodError, parentError);
			glm::vec3 worldCenter = glm::vec3(glm::vec4(cluster.boundingSphereCenter, 1.0));
			//TODO: handle arbitary scaling
			float worldRadius = glm::length(glm::vec4(glm::vec3(cluster.boundingSphereRadius, 0, 0), 0.0));
			//std::cout << cluster.triangleIndices.size() << std::endl;
			//std::cout << cluster.boundingSphereRadius << " " << worldRadius << std::endl;
			ASSERT(cluster.triangleIndices.size() <= CLUSTER_MAX_SIZE, "cluster.triangleIndices.size() is over thresold");
			ASSERT(cluster.boundingSphereRadius > 0 || cluster.triangleIndices.size() == 0, "boundingSphereRadius <= 0");
			ASSERT(worldRadius > 0 || cluster.triangleIndices.size() == 0, "worldRadius <= 0");
			errorInfo[j + currClusterNum].centerR = glm::vec4(worldCenter, worldRadius);
			float parentBoundingRadius = 0;
			glm::vec3 parentCenter = glm::vec3(0);
			if (i == meshes.size() - 1)//last level of lod, no parent
			{
				parentBoundingRadius = worldRadius * 1.5f;
				parentCenter = cluster.boundingSphereCenter;
			}
			/*else
			{
				maxParentBoundingRadius = referenceMesh->meshes[i + 1].clusters[cluster.parentClusterIndices[0]].boundingSphereRadius;
				parentCenter = referenceMesh->meshes[i + 1].clusters[cluster.parentClusterIndices[0]].boundingSphereCenter;
			}*/
			else for (size_t k : cluster.parentClusterIndices)//get max parent bounding sphere size
			{
				parentBoundingRadius = std::max(parentBoundingRadius, meshes[i + 1].clusters[k].boundingSphereRadius);
				parentCenter += meshes[i + 1].clusters[k].boundingSphereCenter;
				//parentCenter = cluster.boundingSphereCenter;
				break;
			}
			//parentCenter /= i == meshes.size() - 1 ? 1.0 : cluster.parentClusterIndices.size();
			glm::vec3 parentWorldCenter = glm::vec3(glm::vec4(parentCenter, 1.0));
			//TODO: handle arbitary scaling

			//assert(parentWorldRadius > 0 && parentWorldRadius > worldRadius);
			errorInfo[j + currClusterNum].centerRP = glm::vec4(parentWorldCenter, parentBoundingRadius);
		}
		currClusterNum += meshes[i].clusterNum;
		currTriangleNum += meshes[i].triangleIndicesSortedByClusterIdx.size();
#ifdef DEBUG_LOD_START
		break;
#endif // DEBUG_LOD_START
	}
}

void loadvkglTFModel(const vkglTF::Model& model, std::vector<NaniteMesh>& naniteMeshes) {
	ASSERT(0, "Not implemented");
}

void packNaniteMeshesToIndexBuffer(const std::vector<NaniteMesh>& naniteMeshes, std::vector<uint32_t>& indexBuffer){

}