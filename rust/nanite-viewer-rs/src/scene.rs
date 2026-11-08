//! Scene assembly: a baked [`Asset`] plus instance transforms -> the GPU payload buffers.
//!
//! This is the Rust port of `mesh/NaniteScene.cpp`, restricted to the branch the Rust asset
//! pipeline can actually produce. The C++ `NaniteScene` has two BVH paths:
//! `linkBlobBVHNodeInfos()` for meshes that came from a `.fcc`, and `createBVHNodeInfos()` which
//! re-flattens an in-memory `shared_ptr` tree built by the OpenMesh builder. A Rust-loaded model is
//! *always* blob-shaped - either read from disk or produced in-process by
//! `nanite_bake_rs::bake_asset` - so only the arithmetic link is needed, and the whole
//! `NaniteBVHNode` object graph never exists here.
//!
//! Ported verbatim, including the quirks:
//! - `createVertexIndexBuffer`: `indexOffset` advances by the **vertex** count, not the index count.
//! - `createClusterInfos`: `clusterIndexCounts` stores an **end index**, not a count.
//! - `linkBlobBVHNodeInfos`: a `VIRTUAL_NODE` is never emitted into `bvhNodeInfos`; its children are
//!   the per-LOD roots, which is exactly what `Asset::bvh_roots` holds.
//! - `worldBounds`: all eight AABB corners are transformed, because an instance may be rotated.

use anyhow::{anyhow, bail, Result};
use glam::{Mat4, Vec3};
use nanite_blob::{node_status, Asset, BlobBvhNode, ClusterInfo, ErrorInfo, Vertex};

use crate::abi::BvhNodeInfo;

/// One instance of one mesh: `mesh::Instance` in `mesh/Instance.h`.
#[derive(Clone, Debug)]
pub struct Instance {
    /// Index into [`Scene::meshes`], i.e. `Instance::referenceMesh`.
    pub mesh: usize,
    /// `Instance::rootTransform`, world space.
    pub root_transform: Mat4,
}

impl Instance {
    pub fn new(mesh: usize, root_transform: Mat4) -> Self {
        Self { mesh, root_transform }
    }
}

/// The fully linked scene, ready to be uploaded. Mirrors the `NaniteScene` members one to one.
/// `Clone` because a resize rebuilds every nanite buffer at the new resolution, and
/// [`crate::renderer::Renderer`] keeps the last [`Scene`] around to re-upload it with.
#[derive(Clone, Default, Debug)]
pub struct Scene {
    /// `naniteObjects`
    pub instances: Vec<Instance>,
    /// `naniteMeshes`: one baked asset per distinct model.
    pub meshes: Vec<Asset>,

    /// `vertices` - the concatenation of every mesh's vertex buffer.
    pub vertices: Vec<Vertex>,
    /// `indices` - the concatenation of every mesh's index buffer, already offset.
    pub indices: Vec<u32>,
    /// `indexOffsets`, per mesh.
    pub index_offsets: Vec<u32>,
    /// `indexCounts`, per mesh.
    pub index_counts: Vec<u32>,

    /// `clusterInfo`
    pub cluster_infos: Vec<ClusterInfo>,
    /// `errorInfo`
    pub error_infos: Vec<ErrorInfo>,
    /// `clusterIndexOffsets`, per mesh: the first scene cluster of that mesh.
    pub cluster_index_offsets: Vec<u32>,
    /// `clusterIndexCounts`, per mesh: the **end** index of that mesh's clusters.
    pub cluster_index_counts: Vec<u32>,

    /// `bvhNodeInfos` - the scene-space BVH the shaders walk.
    pub bvh_node_infos: Vec<BvhNodeInfo>,
    /// `sortedClusterIndices`
    pub sorted_cluster_indices: Vec<u32>,
    /// `depthCounts`
    pub depth_counts: Vec<u32>,
    /// `depthLeafCounts`
    pub depth_leaf_counts: Vec<u32>,
    /// `initNodeInfoIndices`: `[count, 0, 1, .., count-1]`, the BVH roots to start at.
    pub init_node_info_indices: Vec<u32>,
    /// `maxDepthCounts`
    pub max_depth_counts: u32,
    /// `maxClusterNum`
    pub max_cluster_num: u32,
    /// `sceneIndicesCount`
    pub scene_indices_count: u32,
    /// `maxLodLevelNum`
    pub max_lod_level_num: i32,
}

impl Scene {
    /// Build a scene from one baked asset and the instance transforms to draw it with.
    ///
    /// This is `createNaniteSceneInfo()` for the single-mesh case, which is what the viewer uses:
    /// `createScene2()` pushes the same `naniteMesh` twice with two different `rootTransform`s.
    pub fn from_asset(asset: Asset, root_transforms: &[Mat4]) -> Result<Self> {
        let meshes = vec![asset];
        let instances = root_transforms
            .iter()
            .map(|m| Instance::new(0, *m))
            .collect::<Vec<_>>();
        let mut scene = Scene {
            instances,
            meshes,
            max_lod_level_num: -1,
            ..Default::default()
        };
        scene.create_vertex_index_buffer();
        scene.create_cluster_infos()?;
        scene.link_blob_bvh_node_infos()?;
        Ok(scene)
    }

    /// `NaniteScene::createVertexIndexBuffer`.
    fn create_vertex_index_buffer(&mut self) {
        let mut index_offset = 0u32;
        for mesh_index in 0..self.meshes.len() {
            let mesh = &self.meshes[mesh_index];
            self.vertices.extend_from_slice(&mesh.vertices);
            self.indices
                .extend(mesh.indices.iter().map(|i| i + index_offset));
            self.index_offsets.push(index_offset);
            self.index_counts.push(mesh.indices.len() as u32);
            // Verbatim from the C++: the offset advances by the *vertex* count. The index buffer is
            // per-LOD concatenated, so this only lines up because every LOD's vertex range is the
            // same prefix; do not "correct" it to index_counts.
            index_offset += mesh.vertices.len() as u32;
            self.max_lod_level_num = self.max_lod_level_num.max(mesh.lod_nums as i32);
        }
    }

    /// `NaniteScene::createClusterInfos`.
    fn create_cluster_infos(&mut self) -> Result<()> {
        self.cluster_infos.clear();
        self.error_infos.clear();
        self.cluster_index_offsets.clear();
        self.cluster_index_counts.clear();
        self.scene_indices_count = 0;
        self.max_cluster_num = 0;

        for mesh_index in 0..self.meshes.len() {
            let mesh = &self.meshes[mesh_index];
            self.cluster_index_offsets
                .push(self.cluster_infos.len() as u32);

            // The blob's triangle ranges are mesh-local; shift them into the scene index buffer.
            // Verbatim: `indexCounts[meshIndex - 1] / 3` - the *immediately previous* mesh's index
            // count, not the running total. Correct for the two-mesh case the builder reaches and
            // wrong for three or more; kept as-is so both viewers agree.
            let index_shift = if mesh_index != 0 {
                self.index_counts[mesh_index - 1] / 3
            } else {
                0
            };

            for cluster in &mesh.cluster_infos {
                let mut info = *cluster;
                info.triangle_indices_start += index_shift;
                info.triangle_indices_end += index_shift;
                self.cluster_infos.push(info);
            }
            self.error_infos.extend(mesh.error_infos.iter().copied());
            self.cluster_index_counts
                .push(self.cluster_infos.len() as u32);
        }

        if self.cluster_infos.len() != self.error_infos.len() {
            bail!(
                "cluster/error count mismatch: {} clusters, {} error entries",
                self.cluster_infos.len(),
                self.error_infos.len()
            );
        }

        // Per *object*, not per mesh: an instanced model contributes its cluster/index counts once
        // per instance, which is what sizes the raster output buffers.
        for instance in &self.instances {
            // Verbatim: `indexCounts[referenceMeshIndex]` and `clusterIndexCounts[referenceMeshIndex]`,
            // the latter being an **end index** into the scene cluster array rather than that mesh's
            // cluster count. Identical for a single mesh; deliberately not "fixed".
            self.scene_indices_count += self.index_counts[instance.mesh];
            self.max_cluster_num += self.cluster_index_counts[instance.mesh];
        }
        if self.max_cluster_num == 0 {
            bail!("scene has no clusters");
        }
        Ok(())
    }

    /// `NaniteScene::linkBlobBVHNodeInfos`: link every mesh's baked BVHs into one flat,
    /// scene-space array **arithmetically**, with no object graph.
    fn link_blob_bvh_node_infos(&mut self) -> Result<()> {
        // (1) sortedClusterIndices, offset per mesh.
        self.sorted_cluster_indices.clear();
        for mesh_index in 0..self.meshes.len() {
            let offset = self.cluster_index_offsets[mesh_index];
            self.sorted_cluster_indices.extend(
                self.meshes[mesh_index]
                    .sorted_cluster_indices
                    .iter()
                    .map(|c| c + offset),
            );
        }

        // (2) BFS queue of (object, mesh, local node index), seeded with every per-LOD root.
        struct Entry {
            object: usize,
            mesh: usize,
            local: usize,
        }
        let mut queue: Vec<Entry> = Vec::new();
        let mut scene_of_local: Vec<Vec<i32>> = Vec::with_capacity(self.instances.len());
        for (object, instance) in self.instances.iter().enumerate() {
            let mesh = &self.meshes[instance.mesh];
            scene_of_local
                .push(vec![-1; mesh.bvh_nodes.len().max(1)]);
            for root in &mesh.bvh_roots {
                let root = *root as usize;
                if root >= mesh.bvh_nodes.len() {
                    bail!(
                        "bvh root {root} out of range ({} nodes)",
                        mesh.bvh_nodes.len()
                    );
                }
                queue.push(Entry {
                    object,
                    mesh: instance.mesh,
                    local: root,
                });
            }
        }
        if queue.is_empty() {
            bail!("baked asset records no BVH roots");
        }

        // (3) The virtual root is not emitted, so the final count is unknown up front.
        self.bvh_node_infos.clear();
        self.bvh_node_infos.reserve(queue.len() * 2);
        self.depth_counts.clear();
        self.depth_leaf_counts.clear();

        // (4) BFS. `head` advances while the queue grows - the C++ uses a `std::queue`, this uses an
        // index into a growing Vec, which is the same order and avoids a second allocation.
        let mut head = 0usize;
        while head < queue.len() {
            let entry = &queue[head];
            head += 1;
            let object = entry.object;
            let mesh_index = entry.mesh;
            let node: &BlobBvhNode = &self.meshes[mesh_index].bvh_nodes[entry.local];
            let root_transform = self.instances[object].root_transform;

            let scene_index = self.bvh_node_infos.len();
            scene_of_local[object][entry.local] = scene_index as i32;

            let mut info = BvhNodeInfo::default();
            info.p_min_world = transform_point(root_transform, Vec3::from_array(node.p_min));
            info.p_max_world = transform_point(root_transform, Vec3::from_array(node.p_max));
            info.object_id = object as u32;
            info.error_world = node.error_world;

            // `parentBoundingSphere.xyz` is a point, `.w` is the radius.
            let sphere = node.parent_bounding_sphere;
            let sphere_center =
                transform_point(root_transform, Vec3::from_array([sphere[0], sphere[1], sphere[2]]));
            info.error_rp = sphere_center.extend(sphere[3]);

            info.cluster_intervals = [
                (node.start + self.cluster_index_offsets[mesh_index]) as i32,
                (node.end + self.cluster_index_offsets[mesh_index]) as i32,
            ];

            for (j, child) in node.children.iter().enumerate() {
                if *child < 0 {
                    continue;
                }
                let child_local = *child as usize;
                if child_local >= self.meshes[mesh_index].bvh_nodes.len() {
                    bail!(
                        "bvh child {child_local} out of range ({} nodes)",
                        self.meshes[mesh_index].bvh_nodes.len()
                    );
                }
                let mut slot = scene_of_local[object][child_local];
                if slot < 0 {
                    slot = queue.len() as i32;
                    scene_of_local[object][child_local] = slot;
                    queue.push(Entry {
                        object,
                        mesh: mesh_index,
                        local: child_local,
                    });
                }
                info.children[j] = slot;
            }

            if node.depth as usize == self.depth_counts.len() {
                self.depth_counts.push(1);
            } else if (node.depth as usize) < self.depth_counts.len() {
                self.depth_counts[node.depth as usize] += 1;
            } else {
                bail!("bvh node depth {} skips a level", node.depth);
            }

            if node.node_status == node_status::LEAF {
                let clusters_in_node = (node.end - node.start) as i64;
                // Verbatim: the leaf's *worst case* pair count, `n * n`, not `n`.
                let valid = clusters_in_node * clusters_in_node;
                let depth = node.depth as usize;
                if depth >= self.depth_leaf_counts.len() {
                    self.depth_leaf_counts.resize(depth + 1, 0);
                }
                self.depth_leaf_counts[depth] += valid as u32;
            }

            self.bvh_node_infos.push(info);
        }

        // (5) Every real node must be reachable. The virtual root's children are the roots above,
        // so the virtual node itself stays at -1 and is skipped.
        for (object, instance) in self.instances.iter().enumerate() {
            for (i, node) in self.meshes[instance.mesh].bvh_nodes.iter().enumerate() {
                if node.node_status == node_status::VIRTUAL_NODE {
                    continue;
                }
                if scene_of_local[object][i] < 0 {
                    bail!(
                        "baked bvh node {i} of instance {object} is unreachable from the roots \
                         (status {}, lod {})",
                        node.node_status,
                        node.lod_level
                    );
                }
            }
        }

        // (6) The traversal starts from every depth-0 node.
        let roots = self.depth_counts.first().copied().unwrap_or(0);
        self.max_depth_counts = self.depth_counts.iter().copied().max().unwrap_or(0);
        self.init_node_info_indices.clear();
        self.init_node_info_indices.resize(roots as usize + 1, 0);
        self.init_node_info_indices[0] = roots;
        for i in 1..=roots as usize {
            self.init_node_info_indices[i] = (i - 1) as u32;
        }
        Ok(())
    }

    /// `NaniteScene::worldBounds`. Returns `None` for an empty scene.
    ///
    /// This is the only bounds source that exists on both load paths: a blob-only load has no
    /// vkglTF node dimensions to fall back on. The cluster entries are **mesh-local**, so the
    /// per-mesh range is `[clusterIndexOffsets[m], clusterIndexOffsets[m + 1])` - remember that
    /// `cluster_index_counts` stores an end index, not a count.
    pub fn world_bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut any = false;
        for instance in &self.instances {
            let begin = self.cluster_index_offsets[instance.mesh] as usize;
            let end = if instance.mesh + 1 < self.cluster_index_offsets.len() {
                self.cluster_index_offsets[instance.mesh + 1] as usize
            } else {
                self.cluster_infos.len()
            };
            for cluster in &self.cluster_infos[begin..end] {
                let local_min = Vec3::from_array(cluster.p_min_world);
                let local_max = Vec3::from_array(cluster.p_max_world);
                if !(local_min.x <= local_max.x) {
                    continue;
                }
                // All eight corners: a rotated instance's AABB is not spanned by two corners.
                for corner in 0..8u32 {
                    let p = Vec3::new(
                        if corner & 1 == 0 { local_min.x } else { local_max.x },
                        if corner & 2 == 0 { local_min.y } else { local_max.y },
                        if corner & 4 == 0 { local_min.z } else { local_max.z },
                    );
                    let world = transform_point(instance.root_transform, p);
                    min = min.min(world);
                    max = max.max(world);
                    any = true;
                }
            }
        }
        any.then_some((min, max))
    }

    /// Total triangles the cull shaders can ever emit, i.e. the capacity the raster output buffers
    /// need. See the AGENTS.md note: this is a correctness bound, not a memory hint.
    pub fn raster_capacity(&self) -> u32 {
        self.scene_indices_count
    }
}

/// Transform a point, i.e. `(rootTransform * vec4(p, 1)).xyz`.
///
/// Deliberately not `Mat4::transform_point3`: the C++ keeps the `w` row and divides nothing, and a
/// non-affine `rootTransform` would silently change the result. `project_point` below makes the
/// difference visible instead of hiding it.
fn transform_point(m: Mat4, p: Vec3) -> Vec3 {
    let h = m * p.extend(1.0);
    Vec3::new(h.x, h.y, h.z)
}

/// The cluster count of a mesh inside the scene cluster array, for diagnostics.
pub fn mesh_cluster_range(scene: &Scene, mesh_index: usize) -> Option<(usize, usize)> {
    let begin = *scene.cluster_index_offsets.get(mesh_index)? as usize;
    let end = if mesh_index + 1 < scene.cluster_index_offsets.len() {
        scene.cluster_index_offsets[mesh_index + 1] as usize
    } else {
        scene.cluster_infos.len()
    };
    Some((begin, end))
}

/// A one-line summary for the startup / reload log, mirroring the C++ `[reload] ...` line.
pub fn describe(scene: &Scene) -> String {
    let triangles = scene.indices.len() / 3;
    format!(
        "{} triangles, {} clusters, {} bvh nodes, {} instances, {} scene indices, max {} clusters",
        triangles,
        scene.cluster_infos.len(),
        scene.bvh_node_infos.len(),
        scene.instances.len(),
        scene.scene_indices_count,
        scene.max_cluster_num,
    )
}

/// Guard used by tests: a `ClusterInfo` range must stay inside the scene index buffer.
pub fn check_index_ranges(scene: &Scene) -> Result<()> {
    let triangles = (scene.indices.len() / 3) as u32;
    for (i, c) in scene.cluster_infos.iter().enumerate() {
        if c.triangle_indices_end > triangles {
            return Err(anyhow!(
                "cluster {i}: triangle range {}..{} exceeds the scene index buffer ({triangles} triangles)",
                c.triangle_indices_start,
                c.triangle_indices_end
            ));
        }
        if c.triangle_indices_start >= c.triangle_indices_end {
            return Err(anyhow!(
                "cluster {i}: empty or inverted triangle range {}..{}",
                c.triangle_indices_start,
                c.triangle_indices_end
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    /// A minimal two-LOD asset: a virtual root whose two children are the per-LOD leaves, one
    /// cluster each, one triangle per cluster.
    fn toy_asset() -> Asset {
        let vertices = vec![
            Vertex {
                pos: [0.0, 0.0, 0.0],
                ..Default::default()
            },
            Vertex {
                pos: [1.0, 0.0, 0.0],
                ..Default::default()
            },
            Vertex {
                pos: [0.0, 1.0, 0.0],
                ..Default::default()
            },
        ];
        let cluster_infos = vec![
            ClusterInfo {
                p_min_world: [-1.0, -1.0, -1.0],
                p_max_world: [1.0, 1.0, 1.0],
                triangle_indices_start: 0,
                triangle_indices_end: 1,
                object_idx: 0,
            },
            ClusterInfo {
                p_min_world: [0.0, 0.0, 0.0],
                p_max_world: [2.0, 2.0, 2.0],
                triangle_indices_start: 0,
                triangle_indices_end: 1,
                object_idx: 0,
            },
        ];
        let error_infos = vec![
            ErrorInfo {
                error_world: [-1.0, 1.0],
                ..Default::default()
            },
            ErrorInfo {
                error_world: [0.0, 1.0],
                ..Default::default()
            },
        ];
        // Local 0 is the VIRTUAL_NODE the BFS never emits; 1 and 2 are the LOD roots in bvh_roots.
        let bvh_nodes = vec![
            BlobBvhNode {
                children: [1, 2, -1, -1],
                node_status: node_status::VIRTUAL_NODE,
                ..Default::default()
            },
            BlobBvhNode {
                p_min: [-1.0, -1.0, -1.0],
                p_max: [1.0, 1.0, 1.0],
                parent_bounding_sphere: [0.0, 0.0, 0.0, 1.732_050_8],
                error_world: [-1.0, 1.0],
                end: 1,
                node_status: node_status::LEAF,
                lod_level: 0,
                index: 0,
                ..Default::default()
            },
            BlobBvhNode {
                p_min: [0.0, 0.0, 0.0],
                p_max: [2.0, 2.0, 2.0],
                parent_bounding_sphere: [1.0, 1.0, 1.0, 1.732_050_8],
                error_world: [0.0, 1.0],
                start: 1,
                end: 2,
                node_status: node_status::LEAF,
                lod_level: 1,
                index: 0,
                ..Default::default()
            },
        ];
        Asset {
            source_path: "toy".to_string(),
            source_hash: 0,
            lod_nums: 2,
            vertex_stride: Vertex::SIZE as u32,
            vertices,
            indices: vec![0, 1, 2],
            cluster_infos,
            error_infos,
            bvh_nodes,
            sorted_cluster_indices: vec![0, 1],
            lod_cluster_nums: vec![1, 1],
            bvh_roots: vec![1, 2],
        }
    }

    /// Two instances of the toy mesh, the second translated by +10 in x - the shape of
    /// `createScene2()`, which draws the same `naniteMesh` twice.
    fn toy_scene() -> Scene {
        Scene::from_asset(
            toy_asset(),
            &[Mat4::IDENTITY, Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))],
        )
        .expect("toy scene builds")
    }

    #[test]
    fn vertex_index_offsets_use_the_vertex_count() {
        let scene = toy_scene();
        assert_eq!(scene.index_offsets, vec![0]);
        assert_eq!(scene.index_counts, vec![3]);
        assert_eq!(scene.indices, vec![0, 1, 2]);
        assert_eq!(scene.max_lod_level_num, 2);
    }

    #[test]
    fn cluster_index_counts_are_end_indices() {
        let scene = toy_scene();
        assert_eq!(scene.cluster_index_offsets, vec![0]);
        assert_eq!(scene.cluster_index_counts, vec![2]);
        assert_eq!(mesh_cluster_range(&scene, 0), Some((0, 2)));
        // Per *object*: two instances of a 3-index / 2-cluster mesh.
        assert_eq!(scene.scene_indices_count, 6);
        assert_eq!(scene.max_cluster_num, 4);
        assert_eq!(scene.raster_capacity(), 6);
    }

    #[test]
    fn the_virtual_root_is_never_emitted() {
        let scene = toy_scene();
        // Three baked nodes, one of them virtual -> two scene nodes *per instance*, and the toy
        // scene has two instances.
        assert_eq!(scene.bvh_node_infos.len(), 4);
        assert_eq!(scene.depth_counts, vec![4]);
        assert_eq!(scene.max_depth_counts, 4);
        // `[count, 0, 1, ..]` - the C++ initNodeInfosBuffer layout.
        assert_eq!(scene.init_node_info_indices, vec![4, 0, 1, 2, 3]);
        assert_eq!(scene.bvh_node_infos[0].cluster_intervals, [0, 1]);
        assert_eq!(scene.bvh_node_infos[1].cluster_intervals, [1, 2]);
        assert!(scene.bvh_node_infos.iter().all(|n| n.is_leaf()));
    }

    #[test]
    fn sorted_cluster_indices_are_offset_per_mesh() {
        assert_eq!(toy_scene().sorted_cluster_indices, vec![0, 1]);
    }

    #[test]
    fn every_instance_gets_its_own_object_id_and_transformed_bounds() {
        let scene = toy_scene();
        let ids: Vec<u32> = scene.bvh_node_infos.iter().map(|n| n.object_id).collect();
        assert_eq!(ids, vec![0, 0, 1, 1]);
        assert_eq!(scene.bvh_node_infos[0].p_min_world, Vec3::new(-1.0, -1.0, -1.0));
        assert_eq!(scene.bvh_node_infos[2].p_min_world, Vec3::new(9.0, -1.0, -1.0));
        // errorRp is the transformed sphere centre with the radius kept in .w. The BFS seeds the
        // roots in `bvh_roots` order, so index 2 is instance 1's *first* root (local node 1, whose
        // sphere sits at the origin) and index 3 is its second (local node 2, sphere at (1,1,1)).
        assert_eq!(scene.bvh_node_infos[2].error_rp, Vec4::new(10.0, 0.0, 0.0, 1.732_050_8));
        assert_eq!(scene.bvh_node_infos[3].error_rp, Vec4::new(11.0, 1.0, 1.0, 1.732_050_8));
    }

    #[test]
    fn world_bounds_cover_every_instance() {
        let (min, max) = toy_scene().world_bounds().expect("bounds exist");
        // Instance 0 spans [-1,1]^3, instance 1 [9,12] x [0,2] x [0,2].
        assert_eq!(min, Vec3::new(-1.0, -1.0, -1.0));
        assert_eq!(max, Vec3::new(12.0, 2.0, 2.0));
    }

    #[test]
    fn world_bounds_transform_all_eight_corners() {
        // A 45 degree rotation about Y is the case that separates the two strategies: transforming
        // only pMin/pMax gives x >= 0, while the eight corners reach x = -sqrt(2)/2. Both clusters
        // get the same asymmetric box so the second one cannot widen the result by accident.
        let mut asset = toy_asset();
        for cluster in &mut asset.cluster_infos {
            cluster.p_min_world = [0.0, 0.0, 0.0];
            cluster.p_max_world = [4.0, 1.0, 1.0];
        }
        let rotated = Mat4::from_rotation_y(-std::f32::consts::FRAC_PI_4);
        let scene = Scene::from_asset(asset, &[rotated]).expect("rotated scene builds");
        let (min, max) = scene.world_bounds().expect("bounds exist");
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        const S: f32 = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(min.x, -S) && close(max.x, 4.0 * S), "x span {min:?} .. {max:?}");
        assert!(close(min.y, 0.0) && close(max.y, 1.0), "y span {min:?} .. {max:?}");
        assert!(close(min.z, 0.0) && close(max.z, 5.0 * S), "z span {min:?} .. {max:?}");
    }

    #[test]
    fn an_unreachable_real_node_is_reported() {
        let mut asset = toy_asset();
        // Dropping a root leaves its subtree unreachable: that must fail loudly, because a silently
        // missing node means missing geometry.
        asset.bvh_roots = vec![1];
        let err = Scene::from_asset(asset, &[Mat4::IDENTITY])
            .err()
            .expect("must not silently succeed");
        assert!(format!("{err:#}").contains("unreachable"), "{err:#}");
    }

    #[test]
    fn a_missing_root_is_rejected_and_an_empty_asset_has_no_clusters() {
        let mut asset = toy_asset();
        asset.bvh_roots = vec![99];
        assert!(Scene::from_asset(asset, &[Mat4::IDENTITY]).is_err());

        let empty = Asset {
            lod_nums: 1,
            ..Default::default()
        };
        assert!(Scene::from_asset(empty, &[Mat4::IDENTITY]).is_err());
    }

    #[test]
    fn leaf_depth_counts_accumulate_n_squared() {
        // Two leaves covering one cluster each -> 1*1 + 1*1 per instance, the C++ worst-case pair
        // count; the toy scene draws the mesh twice.
        assert_eq!(toy_scene().depth_leaf_counts, vec![4]);
    }

    #[test]
    fn index_ranges_must_stay_inside_the_scene_index_buffer() {
        let scene = toy_scene();
        check_index_ranges(&scene).expect("toy ranges are valid");

        let mut broken = toy_scene();
        broken.cluster_infos[0].triangle_indices_end = 999;
        assert!(check_index_ranges(&broken).is_err());

        let mut inverted = toy_scene();
        inverted.cluster_infos[1].triangle_indices_start =
            inverted.cluster_infos[1].triangle_indices_end;
        assert!(check_index_ranges(&inverted).is_err());
    }

    #[test]
    fn describe_reports_the_scene_shape() {
        let text = describe(&toy_scene());
        assert!(text.contains("1 triangles"), "{text}");
        assert!(text.contains("2 clusters"), "{text}");
        assert!(text.contains("4 bvh nodes"), "{text}");
        assert!(text.contains("2 instances"), "{text}");
    }
}
