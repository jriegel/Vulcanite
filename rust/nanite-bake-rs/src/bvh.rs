//! The per-LOD BVH: construction, error propagation, and flattening into the blob's node array.
//!
//! Ported from `Mesh::buildBVH` + `Mesh::updateBVHErrorCore` (`mesh/Mesh.cpp:873-1160`) and
//! `NaniteMesh::flattenBVH` (`mesh/NaniteMesh.cpp:87-243`). The structure is not incidental -
//! `bvhtraversal.comp` walks exactly this 4-ary layout, and the viewer asserts on the leaf
//! ranges it produces, so the traversal order and the split rule are reproduced literally.
//!
//! # The two error channels
//!
//! Every node and cluster carries `errorWorld = (lodError, parentError)`, and the shaders use
//! them as *gates*:
//!
//! * `bvhtraversal.comp:260` drops a node when `errorWorld.y * screenBound <= threshold`,
//! * `culling.comp:277` drops a cluster when `errorData.x > threshold || errorData.y <= threshold`.
//!
//! A single-LOD blob has no parent level, so both channels are pinned: `lodError = 0` (the level
//! is always good enough) and `parentError = LAST_LOD_PARENT_ERROR` (never refine past it). The
//! C++ builder does the same thing at its last LOD (`NaniteMesh.cpp:822`). Leaving the `-FLT_MAX`
//! defaults in place makes the viewer render *nothing*, which is the single easiest way to get
//! this wrong.

use std::collections::VecDeque;

use glam::Vec3;

use crate::mesh::{BvhNode, LodMesh, NO_RANGE};

/// Arena holding one LOD's BVH. `root` is the index of the per-LOD root node.
#[derive(Clone, Debug, Default)]
pub struct BvhTree {
	pub nodes: Vec<BvhNode>,
	pub root: usize,
}

/// `Mesh::createBVH` = `buildBVH` + `updateBVHError`.
pub fn build_bvh(lod: &mut LodMesh) {
	let mut tree = build_tree(lod);
	update_error(lod, &mut tree);
	lod.bvh = Some(tree);
}

/// `Mesh::buildBVH` (`mesh/Mesh.cpp:873-1067`).
///
/// A LIFO stack DFS over a *permutation array* of cluster-group indices: the node owns a half-open
/// range of that array, and partitioning sorts sub-ranges of it in place. That is why the sort
/// order and the `mid` formulas matter as much as the tree shape - they decide which groups end up
/// in which leaf.
fn build_tree(lod: &mut LodMesh) -> BvhTree {
	let group_num = lod.cluster_groups.len();
	assert!(group_num > 0, "build_bvh: no cluster groups");

	let mut nodes: Vec<BvhNode> = Vec::new();
	let root = nodes.len();
	nodes.push(BvhNode {
		start: 0,
		end: group_num as i64,
		status: nanite_blob::node_status::NODE,
		lod_level: 0,
		..BvhNode::default()
	});

	// `clusterGroupIndex` in C++: the permutation the ranges index into.
	let mut order: Vec<usize> = (0..group_num).collect();
	let mut stack = vec![root];

	while let Some(node_index) = stack.pop() {
		nodes[node_index].lod_level = 0;
		let (start, end, depth, status) = {
			let node = &nodes[node_index];
			(
				node.start as usize,
				node.end as usize,
				node.depth,
				node.status,
			)
		};

		if status == nanite_blob::node_status::LEAF {
			let group = &lod.cluster_groups[start];
			assert!(
				group.clusters.len() <= crate::mesh::Config::CLUSTER_GROUP_MAX_SIZE,
				"cluster group {start} holds {} clusters, over CLUSTER_GROUP_MAX_SIZE",
				group.clusters.len()
			);
			nodes[node_index].cluster_indices = group.clusters.clone();
			nodes[node_index].p_min = group.p_min;
			nodes[node_index].p_max = group.p_max;
			for &cluster in &group.clusters {
				let cluster = &lod.clusters[cluster as usize];
				nodes[node_index].normalized_lod_error = nodes[node_index]
					.normalized_lod_error
					.max(cluster.normalized_lod_error);
				nodes[node_index].parent_normalized_error = nodes[node_index]
					.parent_normalized_error
					.max(cluster.parent_normalized_error);
			}
			continue;
		}

		// Non-leaf: merge the AABBs of the groups in range.
		//
		// **This deliberately uses the direct range, not `order[start..end]`.** `Mesh::buildBVH`
		// merges `clusterGroups[i]` for `i` in `[start, end)` and ignores the permutation entirely
		// (`mesh/Mesh.cpp:963-970`), and the leaf branch does the same with
		// `clusterGroups[currNode->start]` (`:941`). The permutation only decides *how the range is
		// split*, never which group a position stands for - so a node's real content is the
		// position range, and its AABB has to be the union of that range. Merging the permuted
		// groups instead gives a tight AABB around a set of groups that is not the subtree, and
		// `bvhtraversal.comp`'s `frustrumCulling` then drops every leaf outside it: the render
		// collapses to a few scraps. Copy the inconsistency, do not fix it.
		let mut p_min = Vec3::splat(f32::MAX);
		let mut p_max = Vec3::splat(f32::MIN);
		for group in &lod.cluster_groups[start..end] {
			p_min = p_min.min(group.p_min);
			p_max = p_max.max(group.p_max);
		}
		nodes[node_index].p_min = p_min;
		nodes[node_index].p_max = p_max;

		if end - start < 4 {
			// One level above the leaves: stop partitioning and emit one leaf per group.
			for i in start..end {
				let leaf = nodes.len();
				nodes.push(BvhNode {
					status: nanite_blob::node_status::LEAF,
					start: i as i64,
					end: (i + 1) as i64,
					depth: depth + 1,
					lod_level: 0,
					..BvhNode::default()
				});
				nodes[node_index].children.push(leaf);
			}
		} else {
			// BVH4 split: sort the range by the longest axis, cut at `mid`, then sort each half by
			// the second-longest axis and cut again. `diff` is the *merged* extent, so the axis
			// choice is per node, not per group.
			let diff = p_max - p_min;
			let mut longest = 0usize;
			for axis in 1..3 {
				if diff[axis] > diff[longest] {
					longest = axis;
				}
			}
			order[start..end].sort_by(|&a, &b| {
				lod.cluster_groups[a].p_min[longest].total_cmp(&lod.cluster_groups[b].p_min[longest])
			});
			let mid = (start + end) / 2;

			let axis2 = (longest + 1) % 3;
			let axis3 = (longest + 2) % 3;
			let second = if diff[axis2] > diff[axis3] { axis2 } else { axis3 };

			order[start..mid].sort_by(|&a, &b| {
				lod.cluster_groups[a].p_min[second].total_cmp(&lod.cluster_groups[b].p_min[second])
			});
			let mid2 = (start + mid) / 2;

			order[mid..end].sort_by(|&a, &b| {
				lod.cluster_groups[a].p_min[second].total_cmp(&lod.cluster_groups[b].p_min[second])
			});
			let mid3 = (mid + end) / 2;

			for (child_start, child_end) in [(start, mid2), (mid2, mid), (mid, mid3), (mid3, end)] {
				let child = nodes.len();
				nodes.push(BvhNode {
					status: nanite_blob::node_status::NODE,
					start: child_start as i64,
					end: child_end as i64,
					depth: depth + 1,
					lod_level: 0,
					..BvhNode::default()
				});
				nodes[node_index].children.push(child);
			}
		}

		// Pushed in child order, so the LIFO stack visits them right-to-left - the same order the
		// C++ stack does, which is what fixes the BFS indices later on.
		let children = nodes[node_index].children.clone();
		stack.extend(children);
	}

	BvhTree { nodes, root }
}

/// `Mesh::updateBVHError` (`mesh/Mesh.cpp:1070-1160`): bottom-up `parentNormalizedError` and
/// `parentBoundingSphere`.
fn update_error(lod: &mut LodMesh, tree: &mut BvhTree) {
	let root = tree.root;
	let mut error = -f32::MAX;
	let mut sphere = glam::Vec4::ZERO;
	update_error_core(lod, tree, root, &mut error, &mut sphere);
}

/// The C++ signature passes `currNodeError` / `currNodeParentBoundingSphere` **by reference** and
/// initialises them once at the root to `-FLT_MAX` / `(0,0,0,0)`. A non-leaf then does
/// `currNodeError = max(currNodeError, childError)` on that same reference, so a parent's error is
/// the max over its whole subtree *including* whatever its siblings contributed earlier in the
/// recursion. Reproduced by reference for the same reason.
fn update_error_core(
	lod: &mut LodMesh,
	tree: &mut BvhTree,
	node_index: usize,
	curr_node_error: &mut f32,
	curr_node_parent_bounding_sphere: &mut glam::Vec4,
) {
	let status = tree.nodes[node_index].status;

	if status == nanite_blob::node_status::LEAF {
		let mut max_error = -f32::MAX;
		let mut spheres: Vec<glam::Vec4> = Vec::new();
		for &cluster in &tree.nodes[node_index].cluster_indices {
			let cluster = &lod.clusters[cluster as usize];
			spheres.push(glam::Vec4::new(
				cluster.parent_bounding_sphere_center.x,
				cluster.parent_bounding_sphere_center.y,
				cluster.parent_bounding_sphere_center.z,
				cluster.parent_bounding_sphere_radius,
			));
			max_error = max_error.max(cluster.parent_normalized_error as f32);
		}
		*curr_node_error = max_error;
		let sphere = enclose_spheres(&spheres);
		*curr_node_parent_bounding_sphere = sphere;
		let node = &mut tree.nodes[node_index];
		node.parent_normalized_error = *curr_node_error as f64;
		node.parent_bounding_sphere = sphere;
		return;
	}

	let children = tree.nodes[node_index].children.clone();
	let mut spheres: Vec<glam::Vec4> = Vec::with_capacity(children.len());
	for child in children {
		let mut child_error = 0.0f32;
		let mut child_sphere = glam::Vec4::ZERO;
		update_error_core(lod, tree, child, &mut child_error, &mut child_sphere);
		*curr_node_error = curr_node_error.max(child_error);
		spheres.push(child_sphere);
	}
	let sphere = enclose_spheres(&spheres);
	*curr_node_parent_bounding_sphere = sphere;
	let node = &mut tree.nodes[node_index];
	node.parent_bounding_sphere = sphere;
	node.parent_normalized_error = *curr_node_error as f64;
}

/// The "two most distant spheres" rule both branches of `updateBVHErrorCore` use: pick the pair
/// with the largest `distance + r_i + r_j`, then place the enclosing sphere on the line through
/// their centres.
///
/// The `i == j` diagonal is included in the C++ double loop, which is what makes a single-child
/// node work: the best pair is then `(s, s)`, `largestDiameter = 2r`, and the distance test sends
/// it down the "same centre" branch to give back the sphere unchanged.
fn enclose_spheres(spheres: &[glam::Vec4]) -> glam::Vec4 {
	const FLT_EPSILON: f32 = 1.192_092_9e-7;
	if spheres.is_empty() {
		return glam::Vec4::ZERO;
	}

	let mut largest_diameter = f32::MIN;
	let mut sphere1 = glam::Vec4::ZERO;
	let mut sphere2 = glam::Vec4::ZERO;
	for a in spheres {
		for b in spheres {
			let distance = a.truncate().distance(b.truncate());
			let diameter = distance + a.w + b.w;
			if diameter > largest_diameter {
				sphere1 = *a;
				sphere2 = *b;
				largest_diameter = diameter;
			}
		}
	}

	if sphere1.truncate().distance(sphere2.truncate()) < FLT_EPSILON {
		glam::Vec4::new(sphere1.x, sphere1.y, sphere1.z, sphere1.w.max(sphere2.w))
	} else {
		let direction = (sphere2.truncate() - sphere1.truncate()).normalize();
		let center = (sphere1.truncate() + direction * sphere1.w
			+ sphere2.truncate()
			- direction * sphere2.w)
			* 0.5;
		glam::Vec4::new(center.x, center.y, center.z, largest_diameter * 0.5)
	}
}

/// One flattened node - `NaniteBVHNodeInfo`, the POD half that becomes `BlobBvhNode`.
#[derive(Clone, Copy, Debug)]
pub struct FlatNode {
	pub p_min: Vec3,
	pub p_max: Vec3,
	pub parent_bounding_sphere: glam::Vec4,
	/// Builder-local child indices, `-1` padded to four. The virtual root's children are *not*
	/// truncated here - they go into `bvh_roots` instead (`NaniteMesh.cpp:551`).
	pub children: [i32; 4],
	/// `(normalizedlodError, parentNormalizedError)` narrowed to `f32`.
	pub error_world: glam::Vec2,
	/// Leaf: range in `sorted_cluster_indices`. Non-leaf: `0xFFFFFFFF` on both, which is what
	/// `isLeaf = (end - start != 0)` in `bvhtraversal.comp:296` keys on.
	pub start: u32,
	pub end: u32,
	pub depth: i32,
	pub node_status: i32,
	pub lod_level: i32,
	pub index: i32,
}

/// The blob-shaped result of flattening: the virtual root at index 0, the per-LOD roots in
/// `bvh_roots`, and the leaf -> cluster table.
#[derive(Clone, Debug, Default)]
pub struct Flattened {
	pub nodes: Vec<FlatNode>,
	pub bvh_roots: Vec<u32>,
	/// Cluster indices per leaf, in BFS order. A single-mesh blob leaves the LOD offset at 0,
	/// exactly like `clusterIndexOffset` in the C++ builder.
	pub sorted_cluster_indices: Vec<u32>,
}

/// `NaniteMesh::flattenBVH` (`mesh/NaniteMesh.cpp:87-243`): BFS over the virtual root, two passes
/// (assign indices, then write nodes with resolved child indices).
pub fn flatten(tree: &BvhTree) -> Flattened {
	// Pass 1: BFS to assign indices. The virtual root is index 0 and is *not* part of the tree.
	let node_count = tree.nodes.len() + 1;
	let mut flat_index = vec![-1i32; tree.nodes.len()];
	let mut queue: VecDeque<i32> = VecDeque::new();
	queue.push_back(-1); // -1 == the virtual root
	let mut index = 0i32;
	while let Some(current) = queue.pop_front() {
		index += 1;
		if current >= 0 {
			flat_index[current as usize] = index - 1;
		}
		let children = if current < 0 {
			vec![tree.root as i32]
		} else {
			tree.nodes[current as usize].children.iter().map(|&c| c as i32).collect()
		};
		for child in children {
			queue.push_back(child);
		}
	}
	assert!(index == node_count as i32, "flatten: node count mismatch");

	// Pass 2: same BFS, writing the nodes.
	let mut out = Flattened::default();
	let mut queue: VecDeque<i32> = VecDeque::new();
	queue.push_back(-1);
	while let Some(current) = queue.pop_front() {
		let mut info = FlatNode {
			p_min: Vec3::splat(f32::MAX),
			p_max: Vec3::splat(f32::MIN),
			parent_bounding_sphere: glam::Vec4::ZERO,
			children: [-1; 4],
		// `-FLT_MAX` on both channels: `NaniteBVHNode`'s defaults, and what the C++ blob's node 0
		// actually contains. A virtual node is never error-culled - traversal starts at its
		// children - so the value is inert, but it is what a C++-baked blob looks like.
		error_world: glam::Vec2::splat(-f32::MAX),
			start: NO_RANGE as u32,
			end: NO_RANGE as u32,
			depth: 0,
			node_status: nanite_blob::node_status::VIRTUAL_NODE,
			lod_level: -1,
			index: out.nodes.len() as i32,
		};

		let children: Vec<i32> = if current < 0 {
			vec![tree.root as i32]
		} else {
			let node = &tree.nodes[current as usize];
			info.p_min = node.p_min;
			info.p_max = node.p_max;
			info.parent_bounding_sphere = node.parent_bounding_sphere;
			info.error_world = glam::Vec2::new(
				node.normalized_lod_error as f32,
				node.parent_normalized_error as f32,
			);
			info.depth = node.depth;
			info.node_status = node.status;
			info.lod_level = node.lod_level;
			node.children.iter().map(|&c| c as i32).collect()
		};

		if current >= 0 && tree.nodes[current as usize].status == nanite_blob::node_status::LEAF {
			let cluster_indices = &tree.nodes[current as usize].cluster_indices;
			info.start = out.sorted_cluster_indices.len() as u32;
			out.sorted_cluster_indices.extend(cluster_indices);
			info.end = out.sorted_cluster_indices.len() as u32;
		} else {
			for (slot, child) in children.iter().enumerate().take(4) {
				info.children[slot] = flat_index[*child as usize];
			}
			for child in children.iter().copied() {
				queue.push_back(child);
			}
		}

		if current < 0 {
			// The virtual root's children are the per-LOD roots, recorded by flattened index.
			out.bvh_roots = children
				.iter()
				.map(|&c| flat_index[c as usize] as u32)
				.collect();
			// `flattenBVH` fills the virtual root's `children` like any other node and `bakeBlob`
			// then truncates to four. The traversal never reads them (it starts at `bvh_roots`), but
			// a C++-baked blob has them, so keep them.
			for (slot, child) in children.iter().enumerate().take(4) {
				info.children[slot] = flat_index[*child as usize];
			}
		}
		out.nodes.push(info);
	}

	out
}

/// The `ClusterInfo` / `ErrorInfo` arrays - `NaniteMesh::buildClusterInfo` (`mesh/NaniteMesh.cpp:745-855`).
///
/// Two independent orderings meet in this function and mixing them up is the classic failure:
/// `cluster_infos` is indexed by *cluster*, while `triangle_indices_start/end` index into the
/// *face-sorted* index array. The runs are derived from `triangles_sorted_by_cluster`, exactly as
/// the C++ loop does, and `pbrtexture.cpp:3317` asserts `start < end <= indexCount / 3` for every
/// cluster afterwards.
pub fn build_cluster_info(lod: &LodMesh) -> (Vec<nanite_blob::ClusterInfo>, Vec<nanite_blob::ErrorInfo>) {
	let mut cluster_infos = vec![nanite_blob::ClusterInfo::default(); lod.clusters.len()];
	let mut error_infos = vec![nanite_blob::ErrorInfo::default(); lod.clusters.len()];

	// AABBs, merged over every face of the cluster in mesh face order.
	for face in 0..lod.mesh.face_count() as u32 {
		let cluster = lod.triangle_cluster_index[face as usize] as usize;
		let [a, b, c] = lod.mesh.face_vertices(face);
		let min = a.min(b.min(c));
		let max = a.max(b.max(c));
		let info = &mut cluster_infos[cluster];
		info.p_min_world = Vec3::from_array(info.p_min_world).min(min).to_array();
		info.p_max_world = Vec3::from_array(info.p_max_world).max(max).to_array();
	}

	// Triangle ranges from the runs in the sorted face array.
	let mut current_cluster = u32::MAX;
	for (position, &face) in lod.triangles_sorted_by_cluster.iter().enumerate() {
		let cluster = lod.triangle_cluster_index[face as usize];
		if cluster != current_cluster {
			if current_cluster != u32::MAX {
				cluster_infos[current_cluster as usize].triangle_indices_end = position as u32;
			}
			current_cluster = cluster;
			cluster_infos[cluster as usize].triangle_indices_start = position as u32;
		}
	}
	if current_cluster != u32::MAX {
		cluster_infos[current_cluster as usize].triangle_indices_end =
			lod.triangles_sorted_by_cluster.len() as u32;
	}

	for (cluster_index, cluster) in lod.clusters.iter().enumerate() {
		assert!(
			cluster.parent_normalized_error as f32 > cluster.normalized_lod_error as f32,
			"cluster {cluster_index}: parent error must exceed the level's own error"
		);
		assert!(
			cluster.bounding_sphere_radius > 0.0,
			"cluster {cluster_index}: degenerate bounding sphere"
		);
		error_infos[cluster_index].error_world = glam::Vec2::new(
			cluster.normalized_lod_error as f32,
			crate::mesh::LAST_LOD_PARENT_ERROR,
		)
		.to_array();
		error_infos[cluster_index].center_r = glam::Vec4::new(
			cluster.bounding_sphere_center.x,
			cluster.bounding_sphere_center.y,
			cluster.bounding_sphere_center.z,
			cluster.bounding_sphere_radius,
		)
		.to_array();
		// Last LOD has no parent, so the parent sphere is the cluster's own, inflated -
		// `parentBoundingRadius = worldRadius * 1.5f` (`NaniteMesh.cpp:836`).
		error_infos[cluster_index].center_rp = glam::Vec4::new(
			cluster.bounding_sphere_center.x,
			cluster.bounding_sphere_center.y,
			cluster.bounding_sphere_center.z,
			cluster.bounding_sphere_radius * crate::mesh::LAST_LOD_PARENT_RADIUS_FACTOR,
		)
		.to_array();
	}

	(cluster_infos, error_infos)
}
