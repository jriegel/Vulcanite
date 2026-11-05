//! Assembling the finished `.fcc` payload from a built LOD.
//!
//! This is `NaniteMesh::bakeBlob` (`mesh/NaniteMesh.cpp:487-585`) plus
//! `Mesh::initUniqueVertexBuffer` (`mesh/Mesh.cpp:709-721`). The vertex array deliberately keeps
//! the glTF's normals and texture coordinates: the viewer never reads them for a nanite mesh
//! (rasterization works from the visibility buffer and `shading.frag` re-derives everything), but
//! `vertexStride` is `sizeof(vkglTF::Vertex)` and the C++ blob carries them, so a Rust blob that
//! zeroed them would differ from a C++ one for no reason.

use std::path::Path;

use crate::gltf_load::LoadedMesh;
use crate::mesh::{Config, LodMesh};

/// Build the complete blob payload for a single-LOD mesh.
///
/// `lod_nums = 1` is legal and is what a single-level builder must write: `read()` only requires
/// `lodNums > 0`, and the viewer's consumers are a log line, the ImGui slider maximum, and
/// `maxLodLevelNum`, which is stored and never read (`mesh/NaniteScene.cpp:110`).
pub fn build_asset(loaded: &LoadedMesh, lod: &LodMesh, source_path: &Path) -> nanite_blob::Asset {
	assert_eq!(
		lod.mesh.vertex_count(),
		loaded.vertices.len(),
		"emit: mesh and glTF vertex counts disagree"
	);

	let mut asset = nanite_blob::Asset::default();
	// `weakly_canonical` in C++: absolute, normalised, non-existent components resolved. The blob
	// is then re-hashable from any working directory (`AGENTS.md`, "two-stage asset pipeline").
	asset.source_path = canonical(source_path).display().to_string();
	asset.lod_nums = 1;
	asset.vertex_stride = nanite_blob::SIZE_VERTEX;

	// --- VERTICES -------------------------------------------------------------
	for vertex in &loaded.vertices {
		asset.vertices.push(nanite_blob::Vertex {
			pos: vertex.pos.to_array(),
			normal: vertex.normal.to_array(),
			uv: vertex.uv.to_array(),
			color: vertex.color.to_array(),
			// `joint0 = glm::vec4(lodLevel)` - the LOD level rides in `joint0.x`, and the viewer
			// uses it to find each LOD's vertex range.
			joint0: [0.0; 4],
			weight0: [0.0; 4],
			tangent: [0.0; 4],
		});
	}

	// --- INDICES --------------------------------------------------------------
	// `triangleVertexIndicesSortedByClusterIdx` rebased by the running vertex count. With one LOD
	// that offset is 0, but the addition is kept so a multi-LOD port stays correct.
	let vertex_start = 0u32;
	asset.indices = lod
		.vertices_sorted_by_cluster
		.iter()
		.map(|&v| v + vertex_start)
		.collect();

	// --- CLUSTER_INFO / ERROR_INFO --------------------------------------------
	let (cluster_infos, error_infos) = crate::bvh::build_cluster_info(lod);
	asset.cluster_infos = cluster_infos;
	asset.error_infos = error_infos;

	// --- BVH_NODES ------------------------------------------------------------
	let tree = lod
		.bvh
		.as_ref()
		.expect("emit: the BVH was never built for this LOD");
	let flattened = crate::bvh::flatten(tree);
	asset.bvh_nodes = flattened
		.nodes
		.iter()
		.map(|node| nanite_blob::BlobBvhNode {
			p_min: node.p_min.to_array(),
			p_max: node.p_max.to_array(),
			parent_bounding_sphere: node.parent_bounding_sphere.to_array(),
			children: node.children,
			error_world: node.error_world.to_array(),
			start: node.start,
			end: node.end,
			depth: node.depth,
			node_status: node.node_status,
			lod_level: node.lod_level,
			index: node.index,
		})
		.collect();
	asset.bvh_roots = flattened.bvh_roots;

	// --- SORTED_CLUSTER_INDICES / LOD_CLUSTER_NUMS ----------------------------
	asset.sorted_cluster_indices = flattened.sorted_cluster_indices;
	asset.lod_cluster_nums = vec![lod.clusters.len() as u32];

	asset
}

/// `std::filesystem::weakly_canonical` equivalent: make the path absolute and resolve `.`/`..`
/// without requiring the file to exist.
///
/// `std::fs::canonicalize` is deliberately *not* used: on Windows it returns an extended-length
/// path (`\??\C:\...`) that `weakly_canonical` never produces, so the `SOURCE_PATH` a Rust bake
/// records would be spelled differently from a C++ one. Symlink resolution is the only thing
/// skipped, and nothing in the pipeline depends on it - the field is informational, and the
/// `sourceHash` is computed from the path the caller passed, not from this string.
fn canonical(path: &Path) -> std::path::PathBuf {
	let absolute = if path.is_absolute() {
		path.to_path_buf()
	} else {
		std::env::current_dir()
			.unwrap_or_default()
			.join(path)
	};
	let mut out = std::path::PathBuf::new();
	for component in absolute.components() {
		match component {
			std::path::Component::CurDir => {}
			std::path::Component::ParentDir => {
				out.pop();
			}
			other => out.push(other.as_os_str()),
		}
	}
	out
}

/// Structural checks the C++ viewer performs after loading (`pbrtexture.cpp:3317-3319` and the
/// asserts in `NaniteMesh::buildClusterInfo`). Running them here turns a bad bake into an error
/// message instead of a device-side hang or an empty frame.
pub fn check_asset(asset: &nanite_blob::Asset) -> Result<(), String> {
	let triangle_count = (asset.indices.len() / 3) as u32;
	if asset.indices.len() % 3 != 0 {
		return Err(format!("index count {} is not a multiple of 3", asset.indices.len()));
	}
	if asset.cluster_infos.len() != asset.error_infos.len() {
		return Err(format!(
			"clusterInfo count {} != errorInfo count {}",
			asset.cluster_infos.len(),
			asset.error_infos.len()
		));
	}
	if asset.lod_nums == 0 {
		return Err("lodNums is 0".to_string());
	}
	if asset.bvh_nodes.is_empty() || asset.bvh_roots.is_empty() {
		return Err("blob: BVH data missing".to_string());
	}
	if asset.vertex_stride != nanite_blob::SIZE_VERTEX {
		return Err(format!(
			"vertexStride is {}, the viewer requires {}",
			asset.vertex_stride,
			nanite_blob::SIZE_VERTEX
		));
	}

	let mut covered = vec![0u32; asset.cluster_infos.len()];
	for (index, info) in asset.cluster_infos.iter().enumerate() {
		if info.triangle_indices_end > triangle_count {
			return Err(format!(
				"cluster {index}: triangleIndicesEnd {} > indexCount/3 {triangle_count}",
				info.triangle_indices_end
			));
		}
		if info.triangle_indices_start >= info.triangle_indices_end {
			return Err(format!(
				"cluster {index}: empty triangle range {}..{}",
				info.triangle_indices_start, info.triangle_indices_end
			));
		}
		let triangles = info.triangle_indices_end - info.triangle_indices_start;
		if triangles > Config::CLUSTER_MAX_SIZE as u32 {
			return Err(format!(
				"cluster {index}: {triangles} triangles, over CLUSTER_MAX_SIZE {}",
				Config::CLUSTER_MAX_SIZE
			));
		}
		if !(info.p_min_world[0] <= info.p_max_world[0]
			&& info.p_min_world[1] <= info.p_max_world[1]
			&& info.p_min_world[2] <= info.p_max_world[2])
		{
			return Err(format!("cluster {index}: inverted AABB"));
		}
	}

	for &cluster in &asset.sorted_cluster_indices {
		if cluster as usize >= covered.len() {
			return Err(format!("sortedClusterIndices references cluster {cluster}"));
		}
		covered[cluster as usize] += 1;
	}
	for (index, count) in covered.iter().enumerate() {
		if *count != 1 {
			return Err(format!(
				"cluster {index} is referenced {count} times by the BVH leaves (must be exactly 1)"
			));
		}
	}

	for (index, error) in asset.error_infos.iter().enumerate() {
		if error.error_world[1] <= error.error_world[0] {
			return Err(format!(
				"cluster {index}: parentError {} <= lodError {} - the viewer would cull it",
				error.error_world[1], error.error_world[0]
			));
		}
		if error.center_r[3] <= 0.0 {
			return Err(format!("cluster {index}: degenerate bounding sphere"));
		}
	}

	for node in &asset.bvh_nodes {
		if node.node_status == nanite_blob::node_status::LEAF && node.start >= node.end {
			return Err(format!("BVH node {}: leaf with empty range", node.index));
		}
		if node.node_status == nanite_blob::node_status::NODE && node.start != node.end {
			return Err(format!(
				"BVH node {}: non-leaf must have start == end (is {} / {})",
				node.index, node.start, node.end
			));
		}
		if node.node_status == nanite_blob::node_status::NODE
			&& node.error_world[1] <= node.error_world[0]
		{
			return Err(format!(
				"BVH node {}: parentError {} <= lodError {} - the traversal would drop the subtree",
				node.index, node.error_world[1], node.error_world[0]
			));
		}
	}

	for &root in &asset.bvh_roots {
		if root as usize >= asset.bvh_nodes.len() {
			return Err(format!("bvhRoots references node {root}, out of range"));
		}
	}

	// The LOD gate. `culling.comp` keeps a cluster only when
	// `errorData.x <= threshold && errorData.y > threshold`, and `error.comp` builds those from
	// `errorWorld.x/y * getScreenBoundRadiusSq(...)`, i.e. the world error scaled by the cluster's
	// screen footprint. The screen factor is camera dependent, so this is a *necessary* bound
	// rather than a sufficient one - but it is the one that catches the failure mode that matters:
	// a blob that leaves `ErrorInfo::errorWorld` at its default (`-1` / `-FLT_MAX`) or writes
	// `parentError = 0` passes every structural check and then renders as scattered scraps, because
	// the selector drops every cluster. A single-LOD asset sets `parentError = LAST_LOD_PARENT_ERROR`
	// (`1e5`), which clears the viewer's `500 / 1e6` threshold by eight orders of magnitude.
	let threshold = crate::mesh::RENDER_THRESHOLD;
	let mut worst = f32::MAX;
	let mut worst_cluster = 0usize;
	for (index, error) in asset.error_infos.iter().enumerate() {
		if error.error_world[1] < worst {
			worst = error.error_world[1];
			worst_cluster = index;
		}
		if error.error_world[0] > threshold {
			return Err(format!(
				"cluster {index}: lodError {} is above the viewer threshold {threshold} - culling.comp would cull it at any zoom",
				error.error_world[0]
			));
		}
		if !(error.error_world[1] > error.error_world[0]) {
			return Err(format!(
				"cluster {index}: parentError {} <= lodError {} - the LOD selector would drop it",
				error.error_world[1], error.error_world[0]
			));
		}
	}
	if worst <= threshold {
		return Err(format!(
			"cluster {worst_cluster}: parentError {worst} is at or below the viewer threshold {threshold} - the LOD selector would drop it",
		));
	}

	Ok(())
}
