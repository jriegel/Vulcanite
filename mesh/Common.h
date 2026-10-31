#pragma once
#include <cstddef>
#include <string>
#include <OpenMesh/Core/Mesh/TriMesh_ArrayKernelT.hh>
#include <glm/glm.hpp>
#include "Config.h"

// The GPU/storage-buffer POD payloads (BVHNodeInfo, ClusterInfo, ErrorInfo) and
// their layout contract live in GpuTypes.h, which is free of OpenMesh includes
// so the viewer can load a baked asset without the asset pipeline's deps.
#include "GpuTypes.h"

struct MyTraits : public OpenMesh::DefaultTraits
{
	VertexAttributes(OpenMesh::Attributes::Normal |
		OpenMesh::Attributes::TexCoord2D);

	//// TODO: When loading from disk, we can set HalfedgeAttributes to be None to reduce memory usage
	//HalfedgeAttributes(OpenMesh::Attributes::PrevHalfedge| 
	//	OpenMesh::Attributes::Status);
	//HalfedgeTraits
	//{
	//	uint32_t clusterGroupIndex = -1;
	//};
};
typedef OpenMesh::TriMesh_ArrayKernelT<MyTraits> MyMesh;

void writeMyMeshToFile(const MyMesh& mesh, const std::string & filename);
