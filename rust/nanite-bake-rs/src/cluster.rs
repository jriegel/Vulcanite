//! Face -> cluster -> cluster-group construction.
//!
//! # What this replaces
//!
//! The C++ builder partitions the *face adjacency graph* with METIS:
//! `Mesh::buildTriangleGraph` + `METIS_PartGraphKway` (`mesh/Mesh.cpp:245`) produce
//! `triangleClusterIndex`, and `Mesh::buildClusterGraph` + a second `METIS_PartGraphKway`
//! (`:591`) produce `clusterGroupIndex`. METIS is linked `PRIVATE` to the `mesh` library and is
//! not available to Rust at all, so the Rust pipeline uses its own deterministic grower.
//!
//! The consequence is worth stating plainly: the *partition* differs, so a Rust-baked blob is
//! **not** byte-identical to a C++ one and never will be. What has to match is everything the
//! viewer asserts or consumes:
//!
//! * every cluster holds at most `CLUSTER_MAX_SIZE` (64) triangles - `swrasterize.comp` packs the
//!   triangle slot into 6 bits (`triangleID & 0x3F`), so 64 is a hard limit, not a guideline;
//! * `triangleIndicesStart/End` are contiguous runs of `triangleIndicesSortedByClusterIdx`
//!   (`NaniteMesh::buildClusterInfo:800-815`), and `pbrtexture.cpp:3317` asserts
//!   `start < end <= indexCount / 3` for *every* cluster - so no empty cluster may be emitted;
//! * `boundingSphereRadius > 0` for every non-empty cluster (`NaniteMesh.cpp:830`);
//! * every cluster index appears in exactly one BVH leaf (`NaniteMesh.cpp:215`).
//!
//! # The grower
//!
//! A single FIFO wavefront over the adjacency graph, cut every `target` nodes. One region is a
//! connected BFS ball; when it reaches `target` the *same* queue keeps feeding the next region,
//! so regions stay spatially adjacent to their neighbours instead of restarting at an arbitrary
//! node. Deterministic: the seed scan is ascending and the queue order is fixed.

use std::collections::{BTreeMap, VecDeque};

use glam::Vec3;

use crate::mesh::{merge_face_aabb, Cluster, ClusterGroup, LodMesh, NONE};

/// Region assignment over an undirected adjacency list.
///
/// Returns `region[node]` and the number of regions. Regions are connected, hold at most
/// `target` nodes, and together cover every node exactly once.
fn grow_regions(adj: &[Vec<u32>], target: usize) -> (Vec<u32>, usize) {
	let node_count = adj.len();
	assert!(node_count > 0, "grow_regions: empty graph");
	let target = target.max(1);

	// 0 = undiscovered, 1 = queued, 2 = assigned. Keeps the frontier free of duplicates.
	let mut state = vec![0u8; node_count];
	let mut region = vec![NONE; node_count];
	let mut queue: VecDeque<u32> = VecDeque::new();
	let mut assigned = 0usize;
	let mut seed = 0usize;
	let mut region_index = 0u32;

	while assigned < node_count {
		// Resume the wavefront where it stopped; only restart from the lowest unassigned node
		// when a component has been fully consumed.
		if queue_backlog_is_empty(&mut state, &mut queue, &mut seed, node_count) {
			break;
		}
		let mut size = 0usize;
		while size < target {
			let node = match queue.pop_front() {
				Some(n) => n as usize,
				None => break,
			};
			if state[node] == 2 {
				continue;
			}
			state[node] = 2;
			region[node] = region_index;
			assigned += 1;
			size += 1;
			for &next in &adj[node] {
				if state[next as usize] == 0 {
					state[next as usize] = 1;
					queue.push_back(next);
				}
			}
		}
		region_index += 1;
	}

	(region, region_index as usize)
}

/// Drain already-assigned entries off the front of the queue; if it runs dry, seed it with the
/// next unassigned node. Returns true while there is nothing to grow from.
fn queue_backlog_is_empty(
	state: &mut [u8],
	queue: &mut VecDeque<u32>,
	seed: &mut usize,
	node_count: usize,
) -> bool {
	while let Some(&head) = queue.front() {
		if state[head as usize] == 2 {
			queue.pop_front();
		} else {
			return false;
		}
	}
	while *seed < node_count && state[*seed] != 0 {
		*seed += 1;
	}
	if *seed < node_count {
		state[*seed] = 1;
		queue.push_back(*seed as u32);
		return false;
	}
	true
}

/// Face adjacency: the two faces sharing an edge are neighbours, mirroring the edges
/// `Mesh::buildTriangleGraph` adds (`mesh/Mesh.cpp:245-252`).
fn face_adjacency(mesh: &crate::mesh::TriangleMesh) -> Vec<Vec<u32>> {
	let mut adj = vec![Vec::new(); mesh.face_count()];
	for face in 0..mesh.face_count() as u32 {
		for edge in 0..3u32 {
			let neighbor = mesh.face_neighbors[face as usize * 3 + edge as usize];
			if neighbor != NONE && !adj[face as usize].contains(&neighbor) {
				adj[face as usize].push(neighbor);
			}
		}
	}
	adj
}

/// Cluster adjacency: two clusters are neighbours when a face edge joins them, the same relation
/// `Mesh::buildClusterGraph` builds from `triangleClusterIndex` (`mesh/Mesh.cpp:270-278`).
fn cluster_adjacency(
	mesh: &crate::mesh::TriangleMesh,
	face_cluster: &[u32],
	cluster_num: usize,
) -> Vec<Vec<u32>> {
	let mut adj = vec![Vec::new(); cluster_num];
	for face in 0..mesh.face_count() as u32 {
		let a = face_cluster[face as usize];
		for edge in 0..3u32 {
			let neighbor = mesh.face_neighbors[face as usize * 3 + edge as usize];
			if neighbor == NONE {
				continue;
			}
			let b = face_cluster[neighbor as usize];
			if a != b && !adj[a as usize].contains(&b) {
				adj[a as usize].push(b);
				adj[b as usize].push(a);
			}
		}
	}
	adj
}

/// `Mesh::getBoundingSphere` (`mesh/Mesh.cpp:455-548`), ported literally.
///
/// Two sweeps for a diameter pair, then the sphere on that pair, then an expansion pass for
/// vertices outside it. Not the tightest enclosing sphere - and it must not become one: the C++
/// builder's error values and the shader's screen-space bound both assume *this* sphere.
pub fn bounding_sphere(
	mesh: &crate::mesh::TriangleMesh,
	triangle_indices: &[u32],
) -> (Vec3, f32) {
	if triangle_indices.is_empty() {
		return (Vec3::ZERO, 0.0);
	}

	// `px` = first vertex of the first triangle, i.e. `*mesh.fv_begin(face)`.
	let first = mesh.triangles[triangle_indices[0] as usize];
	let px = mesh.vertices[first.v[0] as usize];

	let mut dist2_max = -1.0f32;
	let mut py = px;
	let mut pz = px;
	for &face in triangle_indices {
		for &v in &mesh.triangles[face as usize].v {
			let dist2 = (mesh.vertices[v as usize] - px).length_squared();
			if dist2 > dist2_max {
				dist2_max = dist2;
				py = mesh.vertices[v as usize];
			}
		}
	}

	dist2_max = -1.0;
	for &face in triangle_indices {
		for &v in &mesh.triangles[face as usize].v {
			let dist2 = (mesh.vertices[v as usize] - py).length_squared();
			if dist2 > dist2_max {
				dist2_max = dist2;
				pz = mesh.vertices[v as usize];
			}
		}
	}

	let center = (py + pz) * 0.5;
	let mut radius = dist2_max.max(0.0).sqrt() * 0.5;

	// The sphere on the diameter pair is not guaranteed to contain the cluster, so the C++
	// builder grows the radius to the farthest outside vertex - the centre stays put
	// (`Mesh.cpp:495-513`).
	for &face in triangle_indices {
		for &v in &mesh.triangles[face as usize].v {
			let dist2 = (mesh.vertices[v as usize] - center).length_squared();
			if dist2 > radius * radius {
				radius = dist2.sqrt();
			}
		}
	}

	(center, radius)
}

/// Build `clusters`, `triangle_cluster_index`, `triangles_sorted_by_cluster` and
/// `vertices_sorted_by_cluster` for one LOD - `Mesh::generateCluster` (`mesh/Mesh.cpp:180-255`)
/// minus METIS.
///
/// `triangleIndicesSortedByClusterIdx` is a *stable* sort of the face indices by cluster, which
/// is what makes the `triangleIndicesStart/End` runs in `buildClusterInfo` valid; the Rust sort
/// is stable for the same reason, so the runs are identical in structure.
pub fn build_clusters(lod: &mut LodMesh) {
	let target = crate::mesh::Config::CLUSTER_TARGET_SIZE;
	let face_count = lod.mesh.face_count();
	if face_count == 0 {
		return;
	}

	let adj = face_adjacency(&lod.mesh);
	let (face_cluster, cluster_num) = grow_regions(&adj, target);

	lod.triangle_cluster_index = face_cluster;
	lod.clusters = vec![Cluster::default(); cluster_num];

	// Faces ordered by cluster, ascending - and within one cluster still in face order, because
	// the input order is face order and the sort is stable.
	let mut sorted: Vec<u32> = (0..face_count as u32).collect();
	sorted.sort_by_key(|&face| lod.triangle_cluster_index[face as usize]);
	lod.triangles_sorted_by_cluster = sorted;

	// `clusters[c].triangleIndices` is filled by iterating the faces in mesh order and pushing
	// onto the cluster that owns the face (`Mesh.cpp:237-244`) - so face order inside a cluster.
	for face in 0..face_count as u32 {
		let cluster = lod.triangle_cluster_index[face as usize] as usize;
		lod.clusters[cluster].triangle_indices.push(face);
	}

	// `triangleVertexIndicesSortedByClusterIdx`: the three corners of each sorted face, in
	// `fv_iter` order - which is exactly `Tri::v` order, the order the index buffer arrived in.
	lod.vertices_sorted_by_cluster =
		Vec::with_capacity(lod.triangles_sorted_by_cluster.len() * 3);
	for &face in &lod.triangles_sorted_by_cluster {
		lod.vertices_sorted_by_cluster
			.extend_from_slice(&lod.mesh.triangles[face as usize].v);
	}

	for cluster in &mut lod.clusters {
		// LOD 0 is the source mesh: no error was introduced to get here. `lodError = -1` in C++
		// is a "not computed" marker that only survives in the JSON cache - `buildClusterInfo`
		// would write it straight into `errorWorld.x` and `culling.comp:277` would then cull the
		// cluster, so the blob path uses 0 instead.
		cluster.normalized_lod_error = 0.0;
		cluster.parent_normalized_error = crate::mesh::LAST_LOD_PARENT_ERROR as f64;

		let (center, radius) = bounding_sphere(&lod.mesh, &cluster.triangle_indices);
		cluster.bounding_sphere_center = center;
		cluster.bounding_sphere_radius = radius;
		cluster.parent_bounding_sphere_center = center;
		cluster.parent_bounding_sphere_radius = radius;

		cluster.surface_area = cluster
			.triangle_indices
			.iter()
			.map(|&face| lod.mesh.face_area(face))
			.sum();
	}
}

/// Build `cluster_groups` - `Mesh::generateClusterGroup` (`mesh/Mesh.cpp:591-690`) minus METIS,
/// plus the AABB `Mesh::getClusterGroupAABB` (`:1225`) computes for the BVH.
pub fn build_cluster_groups(lod: &mut LodMesh) {
	let target = crate::mesh::Config::CLUSTER_GROUP_TARGET_SIZE;
	if lod.clusters.is_empty() {
		return;
	}

	let adj = cluster_adjacency(&lod.mesh, &lod.triangle_cluster_index, lod.clusters.len());
	let (group_of_cluster, group_num) = grow_regions(&adj, target);

	lod.cluster_groups = vec![ClusterGroup::default(); group_num];
	for (cluster_index, &group) in group_of_cluster.iter().enumerate() {
		lod.clusters[cluster_index].cluster_group_index = group;
		lod.cluster_groups[group as usize].clusters.push(cluster_index as u32);
	}

	// Per-cluster AABBs, merged per group. `getClusterGroupAABB` reaches this by scanning every
	// face once and merging into `clusterGroupIndex[triangleClusterIndex[face]]` - the same set.
	let mut cluster_aabb: Vec<(Vec3, Vec3)> = Vec::with_capacity(lod.clusters.len());
	for cluster in &lod.clusters {
		let mut p_min = Vec3::splat(f32::MAX);
		let mut p_max = Vec3::splat(f32::MIN);
		for &face in &cluster.triangle_indices {
			merge_face_aabb(&mut p_min, &mut p_max, &lod.mesh, face);
		}
		cluster_aabb.push((p_min, p_max));
	}
	for group in &mut lod.cluster_groups {
		let mut p_min = Vec3::splat(f32::MAX);
		let mut p_max = Vec3::splat(f32::MIN);
		for &cluster in &group.clusters {
			let (min, max) = cluster_aabb[cluster as usize];
			p_min = p_min.min(min);
			p_max = p_max.max(max);
		}
		group.p_min = p_min;
		group.p_max = p_max;
	}
}

/// Sanity check on the partition, mirroring the asserts the C++ builder makes on its own output.
///
/// Run before the BVH is built: every violation here turns into a viewer-side assert
/// (`pbrtexture.cpp:3317`) or a silently dropped triangle (`triangleID & 0x3F`), which is much
/// harder to diagnose from a rendered image.
pub fn check_partition(lod: &LodMesh) -> Result<(), String> {
	let max_cluster = crate::mesh::Config::CLUSTER_MAX_SIZE;
	let total_faces = lod.mesh.face_count();

	let mut seen = BTreeMap::new();
	for (cluster_index, cluster) in lod.clusters.iter().enumerate() {
		if cluster.triangle_indices.is_empty() {
			return Err(format!("cluster {cluster_index} is empty"));
		}
		if cluster.triangle_indices.len() > max_cluster {
			return Err(format!(
				"cluster {cluster_index} has {} triangles, over the {max_cluster} limit",
				cluster.triangle_indices.len()
			));
		}
		if cluster.bounding_sphere_radius <= 0.0 {
			return Err(format!(
				"cluster {cluster_index} has a degenerate bounding sphere"
			));
		}
		for &face in &cluster.triangle_indices {
			if let Some(previous) = seen.insert(face, cluster_index) {
				return Err(format!(
					"face {face} is in both cluster {previous} and cluster {cluster_index}"
				));
			}
		}
	}
	if seen.len() != total_faces {
		return Err(format!(
			"{} of {total_faces} faces are unassigned",
			total_faces - seen.len()
		));
	}

	let mut covered = 0usize;
	for group in &lod.cluster_groups {
		if group.clusters.len() > crate::mesh::Config::CLUSTER_GROUP_MAX_SIZE {
			return Err(format!(
				"cluster group has {} clusters, over the {} limit a BVH leaf can hold",
				group.clusters.len(),
				crate::mesh::Config::CLUSTER_GROUP_MAX_SIZE
			));
		}
		covered += group.clusters.len();
	}
	if covered != lod.clusters.len() {
		return Err(format!(
			"cluster groups cover {covered} clusters, expected {}",
			lod.clusters.len()
		));
	}

	if lod.triangles_sorted_by_cluster.len() != total_faces {
		return Err(format!(
			"triangleIndicesSortedByClusterIdx has {} entries, expected {total_faces}",
			lod.triangles_sorted_by_cluster.len()
		));
	}
	if lod.vertices_sorted_by_cluster.len() != total_faces * 3 {
		return Err("triangleVertexIndicesSortedByClusterIdx length mismatch".to_string());
	}
	Ok(())
}
