set(headers
    "Cluster.h"
    "ClusterGroup.h"
    "Common.h"
    "Config.h"
    "GpuTypes.h"
    "Graph.h"
    "IndexTypes.h"
    "Instance.h"
    "MeshHandler.h"
    "Mesh.h"
    "MetisGraph.h"
    "NaniteBlob.h"
    "NaniteBVH.h"
    "NaniteMesh.h"
    "NaniteScene.h"
    "utils.h"
)

set(sources
    "meshTest.cpp"
    "Cluster.cpp"
    "ClusterGroup.cpp"
    "Common.cpp"
    "Instance.cpp"
    "Mesh.cpp"
    "MeshHandler.cpp"
    "MetisGraph.cpp"
    "NaniteBlob.cpp"
    "NaniteMesh.cpp"
    "NaniteScene.cpp"
    "utils.cpp"
)

list(SORT headers)
list(SORT sources)

source_group(Headers FILES ${headers})
source_group(Sources FILES ${sources})

#include_directories(external/OpenMesh/src)
#include_directories(external/metis/include)
find_package(OpenMesh REQUIRED)
find_package(assimp CONFIG REQUIRED)

set(APPLICATION_NAME mesh)

# Set the path to OpenMesh (change this to your actual path)
# set(OpenMesh_DIR "../external/OpenMesh/build")

# add_executable(${APPLICATION_NAME} ${sources} ${headers})

file(GLOB MESH_SRC ${sources} ${headers})
file(GLOB MESH_HEADERS ${headers})

add_library(${APPLICATION_NAME} STATIC ${MESH_SRC})

target_include_directories(${APPLICATION_NAME} PRIVATE ${OPENMESH_INCLUDE_DIRS})
# metis is PRIVATE: `metis.h` only reaches the builder sources through MetisGraph.h, so the
# include directory no longer leaks into the examples (it used to come in via Graph.h).
target_link_libraries(${APPLICATION_NAME} 
PRIVATE assimp::assimp
  ${OPENMESH_LIBRARIES}
  metis
)