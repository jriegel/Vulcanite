//! The in-memory triangle mesh the builder works on.
//!
//! The C++ builder uses an OpenMesh `TriMesh` with halfedge connectivity
//! (`mesh/Cluster.h`, `mesh/Mesh.cpp`). Rust has no OpenMesh, so the connectivity the builder
//! actually needs is rebuilt here from the index buffer:
//!
//! * face -> vertex, and face -> neighbouring face over each of the three edges,
//! * vertex -> incident faces.
//!
//! That is everything `buildTriangleGraph`, `buildClusterGraph`, `getBoundingSphere` and
//! `buildBVH` consume. Directed halfedges are not needed anywhere in that path.

use glam::{Vec3, Vec4};

/// One triangle of the mesh, in mesh-local face order.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tri {
	/// Indices into [`TriangleMesh::vertices`], in the order they appear in the index buffer.
	///
	/// This order is *not* cosmetic: it is what `triangleVertexIndicesSortedByClusterIdx`
	/// (and therefore the blob's index section) is emitted in, and it decides which edge of a
	/// triangle is shared with which neighbour.
	pub v: [u32; 3],
}

/// A triangle mesh plus the adjacency the cluster builder needs.
#[derive(Clone, Debug, Default)]
pub struct TriangleMesh {
	pub name: String,
	pub vertices: Vec<Vec3>,
	pub triangles: Vec<Tri>,
	/// `face_neighbors[face * 3 + edge]` = the face sharing that edge, or `NONE`.
	///
	/// `edge` is the edge *opposite* vertex `edge`, i.e. the edge between
	/// `v[(edge + 1) % 3]` and `v[(edge + 2) % 3]` - the same convention OpenMesh uses for
	/// `fv_iter(f)[0]` being opposite `fv_iter(f)[1]`, which is what the C++ graph builders
	/// rely on when they skip `fv_iter(i)[0]`.
	pub face_neighbors: Vec<u32>,
	/// `vertex_faces[vertex]` = the faces touching that vertex.
	pub vertex_faces: Vec<Vec<u32>>,
	/// `vertex_neighbors[vertex]` = the other vertices connected to it by an edge.
	pub vertex_neighbors: Vec<Vec<u32>>,
}

/// "No neighbour", matching OpenMesh's invalid halfedge handle.
pub const NONE: u32 = u32::MAX;

impl TriangleMesh {
	/// Build a mesh from a position array and a triangle index buffer, computing adjacency.
	///
	/// Degenerate triangles (a repeated vertex index) are dropped: OpenMesh's `add_face` refuses
	/// them too, and a zero-area triangle makes the bounding-sphere construction divide by zero.
	pub fn new(vertices: Vec<Vec3>, indices: &[u32]) -> Self {
		assert!(
			indices.len() % 3 == 0,
			"index buffer length {} is not a multiple of 3",
			indices.len()
		);

		let mut triangles = Vec::with_capacity(indices.len() / 3);
		for tri in indices.chunks_exact(3) {
			let (a, b, c) = (tri[0], tri[1], tri[2]);
			if a == b || b == c || a == c {
				continue;
			}
			if a as usize >= vertices.len()
				|| b as usize >= vertices.len()
				|| c as usize >= vertices.len()
			{
				panic!(
					"index {} out of range for {} vertices",
					a.max(b).max(c),
					vertices.len()
				);
			}
			triangles.push(Tri { v: [a, b, c] });
		}

		let mut mesh = TriangleMesh {
			name: String::new(),
			vertices,
			triangles,
			face_neighbors: Vec::new(),
			vertex_faces: Vec::new(),
			vertex_neighbors: Vec::new(),
		};
		mesh.build_adjacency();
		mesh
	}

	fn build_adjacency(&mut self) {
		let face_count = self.triangles.len();
		let vertex_count = self.vertices.len();

		self.face_neighbors = vec![NONE; face_count * 3];
		self.vertex_faces = vec![Vec::new(); vertex_count];
		self.vertex_neighbors = vec![Vec::new(); vertex_count];

		// Undirected edge -> the faces using it. Keying on the ordered pair keeps the two
		// orientations of one edge together.
		let mut edge_to_faces: std::collections::BTreeMap<(u32, u32), Vec<(u32, u32)>> =
			std::collections::BTreeMap::new();
		for (face, tri) in self.triangles.iter().enumerate() {
			for edge in 0..3u32 {
				let a = tri.v[(edge as usize + 1) % 3];
				let b = tri.v[(edge as usize + 2) % 3];
				let key = if a < b { (a, b) } else { (b, a) };
				edge_to_faces
					.entry(key)
					.or_default()
					.push((face as u32, edge));
			}
			for i in 0..3 {
				self.vertex_faces[tri.v[i] as usize].push(face as u32);
			}
		}

		for faces in edge_to_faces.values() {
			// Every entry in one bucket uses the same undirected edge, so each pair is already
			// recorded as (face, edge-within-that-face). A manifold edge has exactly two entries;
			// a non-manifold seam is linked pairwise so no adjacency is lost.
			for (i, &(face_a, edge_a)) in faces.iter().enumerate() {
				for &(face_b, edge_b) in faces.iter().skip(i + 1) {
					self.face_neighbors[face_a as usize * 3 + edge_a as usize] = face_b;
					self.face_neighbors[face_b as usize * 3 + edge_b as usize] = face_a;
				}
			}
		}

		for tri in &self.triangles {
			for (i, &a) in tri.v.iter().enumerate() {
				for (j, &b) in tri.v.iter().enumerate() {
					if i == j || a == b {
						continue;
					}
					let list = &mut self.vertex_neighbors[a as usize];
					if !list.contains(&b) {
						list.push(b);
					}
				}
			}
		}
	}

	pub fn face_count(&self) -> usize {
		self.triangles.len()
	}

	pub fn vertex_count(&self) -> usize {
		self.vertices.len()
	}

	/// The three corners of a face, in `v` order.
	#[inline]
	pub fn face_vertices(&self, face: u32) -> [Vec3; 3] {
		let tri = self.triangles[face as usize];
		[
			self.vertices[tri.v[0] as usize],
			self.vertices[tri.v[1] as usize],
			self.vertices[tri.v[2] as usize],
		]
	}

	/// Area of a face, matching OpenMesh's `calc_face_area`.
	#[inline]
	pub fn face_area(&self, face: u32) -> f32 {
		let [a, b, c] = self.face_vertices(face);
		(b - a).cross(c - a).length() * 0.5
	}

	/// Axis-aligned bounds of a face.
	#[inline]
	pub fn face_aabb(&self, face: u32) -> (Vec3, Vec3) {
		let [a, b, c] = self.face_vertices(face);
		let min = a.min(b.min(c));
		let max = a.max(b.max(c));
		(min, max)
	}
}

/// Merge `p_min`/`p_max` with the bounds of `face`, the same operation as `mergeAABB`.
pub fn merge_face_aabb(p_min: &mut Vec3, p_max: &mut Vec3, mesh: &TriangleMesh, face: u32) {
	let (min, max) = mesh.face_aabb(face);
	*p_min = p_min.min(min);
	*p_max = p_max.max(max);
}

/// A cluster: a connected set of faces, the unit the renderer culls and rasterizes.
///
/// Field names mirror `struct Cluster` (`mesh/Cluster.h`) so the C++ and Rust builders can be
/// compared line by line.
#[derive(Clone, Debug, Default)]
pub struct Cluster {
	pub cluster_group_index: u32,
	/// Face indices, in mesh face order.
	pub triangle_indices: Vec<u32>,

	/// `normalizedlodError` in C++: the error introduced *by* this cluster's LOD level.
	///
	/// LOD 0 is the source mesh, so this stays `0` there. It must never be negative in a blob:
	/// `culling.comp` keeps a cluster only when `errorWorld.x <= threshold`, and the JSON
	/// builder's `-1` default is only ever a "not computed yet" marker.
	pub normalized_lod_error: f64,
	/// `parentNormalizedError`: the error of the level *above*, i.e. what is lost by using this
	/// cluster instead of the finer one. A cluster is shown while this is still above the
	/// threshold, so the last LOD must use a large value rather than `0`.
	pub parent_normalized_error: f64,

	pub bounding_sphere_center: Vec3,
	pub bounding_sphere_radius: f32,
	pub parent_bounding_sphere_center: Vec3,
	pub parent_bounding_sphere_radius: f32,

	pub surface_area: f32,
}

/// A cluster group: the unit the BVH is built over, mirroring `struct ClusterGroup`.
///
/// `ClusterGroup::parentClusterIndices` is deliberately absent: it is only read by
/// `NaniteMesh::buildClusterInfo` for a *non-last* LOD, and this builder emits a single LOD.
#[derive(Clone, Debug, Default)]
pub struct ClusterGroup {
	pub clusters: Vec<u32>,
	/// `ClusterGroup::pMin`/`pMax`, filled by `Mesh::getClusterGroupAABB` (`mesh/Mesh.cpp:1225`).
	/// `Mesh::buildBVH` sorts on these, so they are part of the tree's shape, not just a cache.
	pub p_min: Vec3,
	pub p_max: Vec3,
}

/// A BVH node in the builder's tree - `NaniteBVHNode` (`mesh/NaniteBVH.h`), the in-memory one
/// with `f64` errors. This is *never* written to disk; `nanite_blob::BlobBvhNode` is.
///
/// `children` holds indices into the owning [`crate::bvh::BvhTree`] rather than nested nodes: the
/// C++ builder uses `shared_ptr`, but the flattening pass needs stable numeric indices anyway, and
/// an arena gives them for free.
#[derive(Clone, Debug)]
pub struct BvhNode {
	pub p_min: Vec3,
	pub p_max: Vec3,
	/// Defaults to the empty sphere `(0, 0, 0, 0)`, like the C++ default.
	pub parent_bounding_sphere: Vec4,
	pub children: Vec<usize>,
	/// `errorWorld.x` - max `normalizedlodError` of the subtree.
	pub normalized_lod_error: f64,
	/// `errorWorld.y` - max `parentNormalizedError` of the subtree.
	pub parent_normalized_error: f64,
	/// Populated only for leaves, and only up to `CLUSTER_GROUP_MAX_SIZE` entries - the same
	/// fixed-size array the C++ node has.
	pub cluster_indices: Vec<u32>,
	pub depth: i32,
	pub status: i32,
	/// `start`/`end` in the flattened node: the range of `sorted_cluster_indices` covered by
	/// this leaf. Non-leaves keep [`NO_RANGE`], which is what makes the shader's
	/// `isLeaf = (end - start != 0)` test work.
	pub start: i64,
	pub end: i64,
	pub lod_level: i32,
}

/// `NaniteBVHNodeInfo`'s `start = end = -1`, carried into the blob as `0xFFFFFFFF`.
pub const NO_RANGE: i64 = -1;

impl Default for BvhNode {
	fn default() -> Self {
		BvhNode {
			// FLT_MAX / -FLT_MAX, exactly like the C++ default: an empty node that any real
			// bounds merge into.
			p_min: Vec3::splat(f32::MAX),
			p_max: Vec3::splat(f32::MIN),
			parent_bounding_sphere: Vec4::ZERO,
			children: Vec::new(),
			normalized_lod_error: f64::MIN,
			parent_normalized_error: f64::MIN,
			cluster_indices: Vec::new(),
			depth: 0,
			status: nanite_blob::node_status::INVALID,
			start: NO_RANGE,
			end: NO_RANGE,
			lod_level: -1,
		}
	}
}

/// The per-LOD mesh: the geometry plus the clustering and BVH derived from it.
///
/// `Mesh::meshes[]` in C++ holds one of these per LOD. The Rust pipeline builds exactly one
/// (LOD 0), so this is a plain struct rather than a vector.
#[derive(Clone, Debug, Default)]
pub struct LodMesh {
	pub mesh: TriangleMesh,
	pub clusters: Vec<Cluster>,
	pub cluster_groups: Vec<ClusterGroup>,
	/// Faces ordered by cluster index - `triangleIndicesSortedByClusterIdx`.
	pub triangles_sorted_by_cluster: Vec<u32>,
	/// Vertex indices in that same order, three per face -
	/// `triangleVertexIndicesSortedByClusterIdx`, which becomes the blob's index section.
	pub vertices_sorted_by_cluster: Vec<u32>,
	/// Face -> cluster, `triangleClusterIndex`.
	pub triangle_cluster_index: Vec<u32>,
	/// The per-LOD BVH root, `rootBVHNode`.
	pub bvh: Option<crate::bvh::BvhTree>,
}

impl LodMesh {
	pub fn new(mesh: TriangleMesh) -> Self {
		LodMesh {
			mesh,
			..Default::default()
		}
	}
}

/// Rasterization / LOD constants, from `mesh/Config.h`.
pub struct Config;

impl Config {
	/// `NANITE_CLUSTER_SIZE` - the target number of triangles per cluster.
	pub const CLUSTER_TARGET_SIZE: usize = 56;
	/// `NANITE_CLUSTER_MAX_SIZE`.
	pub const CLUSTER_MAX_SIZE: usize = 64;
	/// `NANITE_CLUSTER_GROUP_SIZE` - the target number of clusters per group.
	pub const CLUSTER_GROUP_TARGET_SIZE: usize = 15;
	/// `NANITE_CLUSTER_GROUP_MAX_SIZE`, also the size of a leaf's `clusterIndices` array.
	pub const CLUSTER_GROUP_MAX_SIZE: usize = 32;
}

/// The value used where C++ uses `1e5` for "this is the last level, never refine further"
/// (`Instance.h`, `NaniteMesh.cpp:823`).
pub const LAST_LOD_PARENT_ERROR: f32 = 1.0e5;

/// The parent sphere radius multiplier for a last-LOD cluster, mirroring
/// `parentBoundingRadius = worldRadius * 1.5f`.
pub const LAST_LOD_PARENT_RADIUS_FACTOR: f32 = 1.5;

/// The threshold the viewer uses by default: `thresholdInt / 1e6` with `thresholdInt = 500`
/// (`pbrtexture.cpp`). The ImGui slider ranges over `0..=1000`, and `--threshold` pins it.
///
/// Note that the shaders compare this against the world error *times* `getScreenBoundRadiusSq`
/// (`error.comp:50`), a pixel-count squared distance, so the comparison in `check_asset` is a
/// world-space sanity bound and not the exact shader test.
pub const RENDER_THRESHOLD: f32 = 500.0 / 1.0e6;
