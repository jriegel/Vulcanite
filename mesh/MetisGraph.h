#pragma once

/*
	METIS glue for the asset pipeline.

	This header is deliberately separate from Graph.h: `Graph` is just an adjacency
	map and is part of the shared data model, while `MetisGraph`/`toMetisGraph` drag
	`metis.h` (and therefore the METIS include directory) into every translation unit
	that includes Mesh.h. Keeping it here is what lets `mesh` link METIS privately.
*/

#include <vector>

#include <metis.h>

#include "Graph.h"

struct MetisGraph {
	idx_t nvtxs;
	std::vector<idx_t> xadj;
	std::vector<idx_t> adjncy;
	std::vector<idx_t> adjwgt;
};

void toMetisGraph(const Graph& graph, MetisGraph& metisGraph);
