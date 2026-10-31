#pragma once
#include <unordered_map>
#include <vector>
#include <cstdint>

/*
	Plain adjacency map. This used to `#include <metis.h>` and carry the METIS adapter
	(`MetisGraph`, `toMetisGraph`), which leaked the asset pipeline's third-party include
	directory into every translation unit that includes Mesh.h - the viewer included. That
	adapter now lives in MetisGraph.h, which is only included by the builder sources.
*/
struct Graph {
    std::vector<std::unordered_map<uint32_t, int>> adjList;

    void init(uint32_t size) { adjList.resize(size); };

    void addEdge(uint32_t from, uint32_t to, int cost) { adjList[from][to] = cost; };

    void addEdgeCost(uint32_t from, uint32_t to, int cost) { adjList[from][to] += cost; };
};