#pragma once
#include <cstdint>

/*
	`idx_t` is METIS' index type and the mesh builder's containers have always been
	typed with it. Because those containers live in shared headers (Mesh.h,
	ClusterGroup.h), `metis.h` used to be pulled into every translation unit that
	includes Mesh.h - including the viewer, which never calls METIS.

	Shared headers use this alias instead; builder sources keep `idx_t` by including
	MetisGraph.h. FetchMetis.cmake builds METIS with IDXTYPEWIDTH=32, so the two are
	the same type and nothing changes at the call sites.
*/
using metis_idx_t = int32_t;
