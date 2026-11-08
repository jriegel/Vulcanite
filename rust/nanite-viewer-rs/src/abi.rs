//! Byte-exact mirrors of the GPU structures in `mesh/GpuTypes.h` and of every uniform block in
//! `shaders/glsl/pbrtexture/`.
//!
//! The C++ side memcpy's these structs straight into storage buffers, and the GLSL blocks are
//! `std430`, so the Rust encoder has to reproduce the same offsets rather than a `#[repr(C)]`
//! mirror - `glam::Vec3` is 12 bytes/4-aligned in C++ (`glm::vec3`) but WGSL `vec3<f32>` is
//! 16-byte aligned, and those two rules disagree inside a struct.

use glam::{Mat4, Vec3, Vec4};

/// `sizeof(BVHNodeInfo)` from `mesh/GpuTypes.h`.
pub const SIZE_BVH_NODE_INFO: usize = 112;

/// The *GPU* BVH node. Unlike the on-disk [`nanite_blob::BlobBvhNode`] this carries world-space
/// bounds, an object id and absolute cluster intervals, and it is what `bvhtraversal.comp` reads.
///
/// GLSL `std430` layout (verified against the C++ `static_assert`s):
/// `clusterIntervals` 0, `pMinWorld` 16, `pMaxWorld` 32, `objectId` 44, `errorR` 48, `errorRP` 64,
/// `errorWorld` 80, `childrenNodeIndices` 96, `sizeof` 112.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BvhNodeInfo {
    /// `[start, end)` into the *scene* `sortedClusterIndices`; `(0, 0)` marks a non-leaf.
    pub cluster_intervals: [i32; 2],
    pub p_min_world: Vec3,
    pub p_max_world: Vec3,
    pub object_id: u32,
    /// `(clusterBoundingSphere.xyz, worldRadius)` - unused by the shaders, kept for layout.
    pub error_r: Vec4,
    /// `(parentBoundingSphere.xyz, parentRadius)` - the error-culling bound.
    pub error_rp: Vec4,
    /// `(normalizedLodError, parentNormalizedError)`.
    pub error_world: [f32; 2],
    /// Scene indices of up to four children, `-1` terminated.
    pub children: [i32; 4],
}

impl Default for BvhNodeInfo {
    fn default() -> Self {
        Self {
            cluster_intervals: [-1; 2],
            p_min_world: Vec3::splat(f32::MAX),
            p_max_world: Vec3::splat(f32::MIN),
            object_id: 0,
            error_r: Vec4::ZERO,
            error_rp: Vec4::ZERO,
            error_world: [f32::MAX; 2],
            children: [-1; 4],
        }
    }
}

impl BvhNodeInfo {
    /// `true` when `bvhtraversal.comp` would treat this node as a leaf: it decides that from
    /// `end - start != 0`, so a leaf whose interval is empty is indistinguishable from a node.
    pub fn is_leaf(&self) -> bool {
        self.cluster_intervals[1] - self.cluster_intervals[0] != 0
    }
}

/// Append one `BVHNodeInfo` (112 bytes) to `out`.
pub fn encode_bvh_node_info(node: &BvhNodeInfo, out: &mut Vec<u8>) {
    write_i32(out, node.cluster_intervals[0]);
    write_i32(out, node.cluster_intervals[1]);
    out.extend_from_slice(&[0; 8]);
    write_vec3(out, node.p_min_world);
    out.extend_from_slice(&[0; 4]);
    write_vec3(out, node.p_max_world);
    // `pMaxWorld` ends at 44 and `objectId` is a plain uint, so it sits in the vec3's tail padding.
    write_u32(out, node.object_id);
    write_vec4(out, node.error_r);
    write_vec4(out, node.error_rp);
    write_f32(out, node.error_world[0]);
    write_f32(out, node.error_world[1]);
    out.extend_from_slice(&[0; 8]);
    for c in node.children {
        write_i32(out, c);
    }
    debug_assert_eq!(out.len() % SIZE_BVH_NODE_INFO, 0);
}

/// `sizeof(ClusterInfo)`; the blob already stores this layout, see [`nanite_blob::ClusterInfo`].
pub const SIZE_CLUSTER_INFO: usize = 48;

/// Append one `ErrorInfo` (48 bytes) - `errorWorld` 0, `centerR` 16, `centerRP` 32.
pub fn encode_error_info(error_world: [f32; 2], center_r: Vec4, center_rp: Vec4, out: &mut Vec<u8>) {
    write_f32(out, error_world[0]);
    write_f32(out, error_world[1]);
    out.extend_from_slice(&[0; 8]);
    write_vec4(out, center_r);
    write_vec4(out, center_rp);
    debug_assert_eq!(out.len() % 48, 0);
}

/// `uboCullingMatrices` / `uboMatrices` in `bvhtraversal.comp` and `culling.comp`: five `mat4x4`s,
/// 320 bytes.
pub const SIZE_CULLING_MATRICES: usize = 320;

pub fn encode_culling_matrices(
    model: Mat4,
    last_view: Mat4,
    last_proj: Mat4,
    curr_view: Mat4,
    curr_proj: Mat4,
    out: &mut Vec<u8>,
) {
    for m in [model, last_view, last_proj, curr_view, curr_proj] {
        write_mat4(out, m);
    }
    debug_assert_eq!(out.len() % SIZE_CULLING_MATRICES, 0);
}

/// The second uniform block of `bvhtraversal.comp` and the whole of `error.comp`:
/// `view`, `proj`, `camUp`, `camRight` - 160 bytes (`vec3` is 12 bytes with 16-byte alignment).
pub const SIZE_ERROR_MATRICES: usize = 160;

pub fn encode_error_matrices(view: Mat4, proj: Mat4, cam_up: Vec3, cam_right: Vec3, out: &mut Vec<u8>) {
    write_mat4(out, view);
    write_mat4(out, proj);
    write_vec3(out, cam_up);
    out.extend_from_slice(&[0; 4]);
    write_vec3(out, cam_right);
    out.extend_from_slice(&[0; 4]);
    debug_assert_eq!(out.len() % SIZE_ERROR_MATRICES, 0);
}

/// `uboshading` in `shading.frag` / `depthcopy.comp`: `invView`, `invProj`, `camPos` - 144 bytes.
pub const SIZE_SHADING_UBO: usize = 144;

pub fn encode_shading_ubo(inv_view: Mat4, inv_proj: Mat4, cam_pos: Vec3, out: &mut Vec<u8>) {
    write_mat4(out, inv_view);
    write_mat4(out, inv_proj);
    write_vec3(out, cam_pos);
    out.extend_from_slice(&[0; 4]);
    debug_assert_eq!(out.len() % SIZE_SHADING_UBO, 0);
}

/// `DrawInfo` of `hwrasterize.geom` and `swrasterize.comp`: one `uint`, padded to 16 bytes.
pub const SIZE_DRAW_INFO: usize = 16;

pub fn encode_draw_info(num_indices: u32, out: &mut Vec<u8>) {
    write_u32(out, num_indices);
    out.extend_from_slice(&[0; 12]);
}

/// `uboMatrices` of `skybox.vert` / `hwrasterize` / `swrasterize.comp`:
/// `projection`, `model`, `view`, `camPos` - 208 bytes. The same block backs the
/// skybox pass, the hardware rasterize pass and the software rasterizer's
/// `DrawMatrices`. Note this is *not* the 144-byte [`SIZE_SHADING_UBO`]: three
/// `mat4`s plus a `vec3` is 208 under `std140`, not 144.
pub const SIZE_UBO_MATRICES: usize = 208;

/// `vkglTF::Vertex` (`base/VulkanglTFModel.h`) and the blob's `SECTION_VERTICES` element:
/// `pos` 0, `normal` 16, `uv` 32, `color` 48, `joint0` 64, `weight0` 80, `tangent` 96 - **112**
/// bytes. The three `alignas` gaps after `pos`, `normal` and `uv` are what makes it 112 and not
/// 96; the WGSL `Vertex` in `common.wgsl` spells the same gaps out as `_pad*` members.
pub const SIZE_VERTEX: usize = 112;

pub fn encode_vertex(v: &nanite_blob::Vertex, out: &mut Vec<u8>) {
    write_vec3(out, Vec3::from_array(v.pos));
    out.extend_from_slice(&[0; 4]);
    write_vec3(out, Vec3::from_array(v.normal));
    out.extend_from_slice(&[0; 4]);
    write_f32(out, v.uv[0]);
    write_f32(out, v.uv[1]);
    out.extend_from_slice(&[0; 8]);
    for f in [v.color, v.joint0, v.weight0] {
        for c in f {
            write_f32(out, c);
        }
    }
    for c in v.tangent {
        write_f32(out, c);
    }
    debug_assert_eq!(out.len() % SIZE_VERTEX, 0);
}

pub fn encode_ubo_matrices(projection: Mat4, model: Mat4, view: Mat4, cam_pos: Vec3, out: &mut Vec<u8>) {
    write_mat4(out, projection);
    write_mat4(out, model);
    write_mat4(out, view);
    write_vec3(out, cam_pos);
    out.extend_from_slice(&[0; 4]);
    debug_assert_eq!(out.len() % SIZE_UBO_MATRICES, 0);
}

/// `UboParams` of `shading.frag` / `skybox.frag`: four light positions, `exposure`, `gamma`.
///
/// The *data* is 72 bytes, but a uniform block's size is rounded up to the alignment of its
/// largest member (16, from `vec4` / `alignas(16)`), so `sizeof(UBOParams)` in C++ is **80** and
/// naga's uniform-layout check demands the same. Binding a 72-byte buffer fails with
/// "Binding size 72 of Buffer with 'ubo params' label is less than minimum 80".
pub const SIZE_UBO_PARAMS: usize = 80;

pub fn encode_ubo_params(lights: [Vec4; 4], exposure: f32, gamma: f32, out: &mut Vec<u8>) {
    for l in lights {
        write_vec4(out, l);
    }
    write_f32(out, exposure);
    write_f32(out, gamma);
    out.extend_from_slice(&[0; 8]);
    debug_assert_eq!(out.len() % SIZE_UBO_PARAMS, 0);
}

/// `HWRDrawIndexedIndirect` of `culling.comp` / `hwrasterize`: the same 20 bytes as
/// `VkDrawIndexedIndirectCommand`, with `indexCount` an `atomic uint` in the shader.
pub const SIZE_HWR_DRAW_INDIRECT: usize = 20;

pub fn encode_hwr_draw_indirect(index_count: u32, instance_count: u32, out: &mut Vec<u8>) {
    for v in [
        index_count,
        instance_count,
        0, // firstIndex
        0, // vertexOffset
        0, // firstInstance
    ] {
        write_u32(out, v);
    }
    debug_assert_eq!(out.len() % SIZE_HWR_DRAW_INDIRECT, 0);
}

/// `SWRIndirectBuffer` of `clearImage.comp`: the `glDispatchComputeIndirect` argument triple,
/// 12 bytes. `x` is the number of 32-wide workgroups over the software-rasterized triangles.
pub const SIZE_SWR_DISPATCH: usize = 12;

pub fn encode_swr_dispatch(x: u32, y: u32, z: u32, out: &mut Vec<u8>) {
    for v in [x, y, z] {
        write_u32(out, v);
    }
    debug_assert_eq!(out.len() % SIZE_SWR_DISPATCH, 0);
}

/// The four point lights `updateParams()` places at `p = 15`.
pub fn default_lights() -> [Vec4; 4] {
    let p = 15.0;
    [
        Vec4::new(-p, -p * 0.5, -p, 1.0),
        Vec4::new(-p, -p * 0.5, p, 1.0),
        Vec4::new(p, -p * 0.5, p, 1.0),
        Vec4::new(p, -p * 0.5, -p, 1.0),
    ]
}

// ---------------------------------------------------------------------------
// primitive writers
// ---------------------------------------------------------------------------

fn write_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_vec3(out: &mut Vec<u8>, v: Vec3) {
    for c in v.to_array() {
        write_f32(out, c);
    }
}

fn write_vec4(out: &mut Vec<u8>, v: Vec4) {
    for c in v.to_array() {
        write_f32(out, c);
    }
}

/// Column-major, the same order `glm` hands to `vkCmdUpdateBuffer`.
fn write_mat4(out: &mut Vec<u8>, m: Mat4) {
    for col in m.to_cols_array().chunks(4) {
        for c in col {
            write_f32(out, *c);
        }
    }
}

/// `alignUp(v, 256)` - the blob payload alignment, also handy for buffer offsets.
pub fn align_up(v: usize, align: usize) -> usize {
    (v + align - 1) / align * align
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32_at(buf: &[u8], at: usize) -> f32 {
        f32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
    }

    fn offsets_of_bvh_node() -> Vec<(&'static str, usize)> {
        let mut buf = Vec::new();
        encode_bvh_node_info(&BvhNodeInfo::default(), &mut buf);
        // A node with recognisable values in every field, so the offsets below are unambiguous.
        let node = BvhNodeInfo {
            cluster_intervals: [11, 12],
            p_min_world: Vec3::new(13.0, 14.0, 15.0),
            p_max_world: Vec3::new(16.0, 17.0, 18.0),
            object_id: 19,
            error_r: Vec4::new(20.0, 21.0, 22.0, 23.0),
            error_rp: Vec4::new(24.0, 25.0, 26.0, 27.0),
            error_world: [28.0, 29.0],
            children: [30, 31, 32, 33],
        };
        buf.clear();
        encode_bvh_node_info(&node, &mut buf);
        assert_eq!(buf.len(), SIZE_BVH_NODE_INFO);
        let u32_at = |at: usize| u32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
        let i32_at = |at: usize| i32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
        // Float fields are compared as their *value*, not their bit pattern - the point of the
        // table is which offset holds which field, and reading a float as a u32 hides that.
        let f32_at = |at: usize| f32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
        vec![
            ("clusterIntervals.x", i32_at(0) as usize),
            ("clusterIntervals.y", i32_at(4) as usize),
            ("pMinWorld.x", f32_at(16)),
            ("pMinWorld.y", f32_at(20)),
            ("pMinWorld.z", f32_at(24)),
            ("pMaxWorld.x", f32_at(32)),
            ("pMaxWorld.y", f32_at(36)),
            ("pMaxWorld.z", f32_at(40)),
            ("objectId", u32_at(44) as usize),
            ("errorR.x", f32_at(48)),
            ("errorR.w", f32_at(60)),
            ("errorRP.x", f32_at(64)),
            ("errorRP.w", f32_at(76)),
            ("errorWorld.x", f32_at(80)),
            ("errorWorld.y", f32_at(84)),
            ("children[0]", i32_at(96) as usize),
            ("children[3]", i32_at(108) as usize),
        ]
    }

    #[test]
    fn bvh_node_info_offsets_match_gpu_types_h() {
        let got = offsets_of_bvh_node();
        let want: Vec<(&str, usize)> = vec![
            ("clusterIntervals.x", 11),
            ("clusterIntervals.y", 12),
            ("pMinWorld.x", 13),
            ("pMinWorld.y", 14),
            ("pMinWorld.z", 15),
            ("pMaxWorld.x", 16),
            ("pMaxWorld.y", 17),
            ("pMaxWorld.z", 18),
            ("objectId", 19),
            ("errorR.x", 20),
            ("errorR.w", 23),
            ("errorRP.x", 24),
            ("errorRP.w", 27),
            ("errorWorld.x", 28),
            ("errorWorld.y", 29),
            ("children[0]", 30),
            ("children[3]", 33),
        ];
        assert_eq!(got, want);
    }

    /// Every encoder appends exactly one block, so each check starts from an empty buffer.
    #[test]
    fn uniform_blocks_have_the_std430_sizes() {
        let mut buf = Vec::new();
        let mut size_of = |encode: &mut dyn FnMut(&mut Vec<u8>)| -> usize {
            buf.clear();
            encode(&mut buf);
            buf.len()
        };
        assert_eq!(size_of(&mut |b| encode_culling_matrices(Mat4::IDENTITY, Mat4::IDENTITY, Mat4::IDENTITY, Mat4::IDENTITY, Mat4::IDENTITY, b)), SIZE_CULLING_MATRICES);
        assert_eq!(size_of(&mut |b| encode_error_matrices(Mat4::IDENTITY, Mat4::IDENTITY, Vec3::X, Vec3::Y, b)), SIZE_ERROR_MATRICES);
        assert_eq!(size_of(&mut |b| encode_shading_ubo(Mat4::IDENTITY, Mat4::IDENTITY, Vec3::X, b)), SIZE_SHADING_UBO);
        assert_eq!(size_of(&mut |b| encode_ubo_matrices(Mat4::IDENTITY, Mat4::IDENTITY, Mat4::IDENTITY, Vec3::X, b)), SIZE_UBO_MATRICES);
        assert_eq!(size_of(&mut |b| encode_ubo_params(default_lights(), 4.5, 2.2, b)), SIZE_UBO_PARAMS);
        assert_eq!(
            size_of(&mut |b| encode_vertex(&nanite_blob::Vertex::default(), b)),
            SIZE_VERTEX
        );
        assert_eq!(size_of(&mut |b| encode_error_info([1.0, 2.0], Vec4::ZERO, Vec4::ZERO, b)), 48);
        assert_eq!(size_of(&mut |b| encode_draw_info(7, b)), SIZE_DRAW_INFO);
        assert_eq!(size_of(&mut |b| encode_hwr_draw_indirect(3, 2, b)), SIZE_HWR_DRAW_INDIRECT);
        assert_eq!(size_of(&mut |b| encode_swr_dispatch(1, 1, 1, b)), SIZE_SWR_DISPATCH);
        assert_eq!(size_of(&mut |b| encode_bvh_node_info(&BvhNodeInfo::default(), b)), SIZE_BVH_NODE_INFO);
    }

    /// The `std140` offsets of the uniform blocks, pinned by value rather than by name. A `vec3`
    /// pad member is 16-byte aligned, so padding with one silently slides the next field - which
    /// is exactly the mistake that puts `camPos` at 160 instead of 144.
    #[test]
    fn uniform_block_field_offsets() {
        let mut buf = Vec::new();

        // ErrorMatrices: view 0, proj 64, camUp 128, camRight 144.
        encode_error_matrices(Mat4::from_cols_array(&[7.0; 16]), Mat4::from_cols_array(&[8.0; 16]), Vec3::new(1.5, 2.5, 3.5), Vec3::new(4.5, 5.5, 6.5), &mut buf);
        assert_eq!(buf.len(), SIZE_ERROR_MATRICES);
        assert_eq!(f32_at(&buf, 0), 7.0);
        assert_eq!(f32_at(&buf, 64), 8.0);
        assert_eq!(f32_at(&buf, 128), 1.5);
        assert_eq!(f32_at(&buf, 144), 4.5);

        // UboMatrices: projection 0, model 64, view 128, camPos 192.
        buf.clear();
        encode_ubo_matrices(Mat4::from_cols_array(&[1.0; 16]), Mat4::from_cols_array(&[2.0; 16]), Mat4::from_cols_array(&[3.0; 16]), Vec3::new(9.0, 10.0, 11.0), &mut buf);
        assert_eq!(buf.len(), SIZE_UBO_MATRICES);
        assert_eq!(f32_at(&buf, 0), 1.0);
        assert_eq!(f32_at(&buf, 64), 2.0);
        assert_eq!(f32_at(&buf, 128), 3.0);
        assert_eq!(f32_at(&buf, 192), 9.0);

        // UboShading: invView 0, invProj 64, camPos 128.
        buf.clear();
        encode_shading_ubo(Mat4::from_cols_array(&[5.0; 16]), Mat4::from_cols_array(&[6.0; 16]), Vec3::new(12.0, 13.0, 14.0), &mut buf);
        assert_eq!(buf.len(), SIZE_SHADING_UBO);
        assert_eq!(f32_at(&buf, 0), 5.0);
        assert_eq!(f32_at(&buf, 64), 6.0);
        assert_eq!(f32_at(&buf, 128), 12.0);

        // UboParams: lights 0..64, exposure 64, gamma 68.
        buf.clear();
        encode_ubo_params(default_lights(), 4.5, 2.2, &mut buf);
        assert_eq!(buf.len(), SIZE_UBO_PARAMS);
        assert_eq!(f32_at(&buf, 64), 4.5);
        assert_eq!(f32_at(&buf, 68), 2.2);
    }

    #[test]
    fn mat4_is_column_major() {
        let mut buf = Vec::new();
        let m = Mat4::from_cols_array(&[
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
        ]);
        write_mat4(&mut buf, m);
        assert_eq!(buf.len(), 64);
        let f = |at: usize| f32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
        // Column 0 first, and the last column's first element at byte 48 - that is `glm`'s order.
        assert_eq!(f(0), 1.0);
        assert_eq!(f(4), 2.0);
        assert_eq!(f(12), 4.0);
        assert_eq!(f(48), 13.0);
        assert_eq!(f(52), 14.0);
        assert_eq!(f(60), 16.0);
    }

    #[test]
    fn align_up_rounds_to_payload_alignment() {
        assert_eq!(align_up(64 + 24 * 9, 256), 512);
        assert_eq!(align_up(256, 256), 256);
        assert_eq!(align_up(1, 256), 256);
    }
}
