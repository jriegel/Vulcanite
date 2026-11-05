//! `nanite-blob` - reader and writer for the Vulcanite Nanite cluster blob (`.fcc`) asset format.
//!
//! This is a byte-exact Rust port of `mesh/NaniteBlob.{h,cpp}`. The blob is the contract between
//! the asset pipeline (the C++ `mesh` library, or `nanite-bake-rs`) and the Vulkan viewer: it
//! stores the *final* GPU payloads in exactly the layout the std430 storage buffers expect, so the
//! viewer uploads them with a straight `memcpy` and never links OpenMesh or METIS.
//!
//! File layout:
//!
//! ```text
//! +-------------------------------+
//! | Header                        |  64 bytes: magic + versions + hashes + struct sizes
//! +-------------------------------+
//! | Section x N                   |  24 bytes each: key, count, offset, size
//! +-------------------------------+
//! | padding to a 256 byte boundary|
//! +-------------------------------+
//! | section payloads              |  raw POD arrays, each 256-byte aligned
//! +-------------------------------+
//! ```
//!
//! All offsets are absolute byte offsets from the start of the file. Every struct below is encoded
//! by hand (little endian, explicit padding) rather than by `#[repr(C)]` + `bytemuck`, because the
//! C++ side uses `alignas` on individual members and the padding must land exactly where the C++
//! compiler puts it. The unit tests pin every size and offset against a blob baked by the C++
//! builder.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// Format constants (mirrors mesh/NaniteBlob.h)
// ---------------------------------------------------------------------------

/// Bump when a struct's *byte layout* changes.
pub const NANITE_BLOB_VERSION: u32 = 2;
/// Bump when the *meaning* of a section changes but its byte layout does not.
pub const NANITE_BUILDER_VERSION: u32 = 1;
/// `'NADB'` = Nanite Data Blob.
pub const NANITE_BLOB_MAGIC: u32 = 0x4244_414E;
/// File extension of a baked blob.
pub const NANITE_BLOB_EXTENSION: &str = ".fcc";

/// Builder parameters folded into [`config_hash`], see `mesh/Config.h`.
pub const CLUSTER_TARGET_SIZE: u32 = 56;
pub const CLUSTER_MAX_SIZE: u32 = 64;
pub const CLUSTER_GROUP_TARGET_SIZE: u32 = 15;
pub const CLUSTER_GROUP_MAX_SIZE: u32 = 32;

/// Identifies a payload array inside the blob.
pub mod section {
    /// `vkglTF::Vertex[]` - unique vertices, LOD0..LODn concatenated.
    pub const VERTICES: u32 = 0;
    /// `uint32_t[]` - triangle vertex indices, cluster-sorted.
    pub const INDICES: u32 = 1;
    /// `ClusterInfo[]` - mesh-relative triangle ranges.
    pub const CLUSTER_INFO: u32 = 2;
    /// `ErrorInfo[]` - per-cluster projected error + bounding spheres.
    pub const ERROR_INFO: u32 = 3;
    /// `BlobBVHNode[]` - per-mesh flattened BVH, with *local* child indices.
    pub const BVH_NODES: u32 = 4;
    /// `uint32_t[]` - leaf -> cluster, offset by the per-LOD cluster base.
    pub const SORTED_CLUSTER_INDICES: u32 = 5;
    /// `uint32_t[]` - cluster count of each LOD.
    pub const LOD_CLUSTER_NUMS: u32 = 6;
    /// `uint32_t[]` - local indices of the virtual root's children (one per LOD).
    pub const BVH_ROOTS: u32 = 7;
    /// `char[]` - source mesh path, UTF-8, no NUL terminator. Informational only.
    pub const SOURCE_PATH: u32 = 8;
    /// Number of known section keys.
    pub const COUNT: usize = 9;
}

/// `NaniteBVHNodeStatus`.
pub mod node_status {
    pub const INVALID: i32 = 0;
    /// The builder's root, whose children are the per-LOD roots. The BFS never emits it into the
    /// scene, but it *is* present in the blob as node 0.
    pub const VIRTUAL_NODE: i32 = 1;
    pub const NODE: i32 = 2;
    pub const LEAF: i32 = 3;
}

// Struct sizes recorded in the header. A blob whose recorded sizes differ is rejected rather than
// silently misread.
pub const SIZE_BVH_NODE_INFO: u32 = 112;
pub const SIZE_CLUSTER_INFO: u32 = 48;
pub const SIZE_ERROR_INFO: u32 = 48;
pub const SIZE_BLOB_BVH_NODE: u32 = 96;
/// `sizeof(vkglTF::Vertex)` - recorded per blob as `vertexStride`.
pub const SIZE_VERTEX: u32 = 112;

const HEADER_SIZE: usize = 64;
const SECTION_SIZE: usize = 24;
/// Every payload starts on a 256 byte boundary.
pub const PAYLOAD_ALIGNMENT: u64 = 256;

// ---------------------------------------------------------------------------
// Hashing (mirrors NaniteBlob::fnv1a / fnv1aFile / configHash)
// ---------------------------------------------------------------------------

/// FNV-1a 64 bit offset basis, also the default seed.
pub const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over `data`, starting from `seed`.
pub fn fnv1a(data: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

/// FNV-1a over a file, in 64 KiB chunks with the seed chained through them.
///
/// A missing file hashes to **0**, and 0 makes [`read`] skip the strict source check on purpose:
/// a shipped `.fcc` whose source mesh has been deleted still loads.
pub fn fnv1a_file(path: &Path) -> u64 {
    let Ok(file) = File::open(path) else {
        return 0;
    };
    let mut reader = std::io::BufReader::new(file);
    let mut hash = FNV1A_OFFSET_BASIS;
    let mut buffer = vec![0u8; 65536];
    loop {
        let Ok(read) = reader.read(&mut buffer) else {
            break;
        };
        if read == 0 {
            break;
        }
        hash = fnv1a(&buffer[..read], hash);
    }
    hash
}

/// Hash of every builder parameter that changes the produced data. A blob baked with different
/// parameters is stale and is rejected.
pub fn config_hash() -> u64 {
    let params = [
        NANITE_BUILDER_VERSION,
        CLUSTER_TARGET_SIZE,
        CLUSTER_MAX_SIZE,
        CLUSTER_GROUP_TARGET_SIZE,
        CLUSTER_GROUP_MAX_SIZE,
    ];
    let mut bytes = Vec::with_capacity(params.len() * 4);
    for p in params {
        bytes.extend_from_slice(&p.to_le_bytes());
    }
    fnv1a(&bytes, FNV1A_OFFSET_BASIS)
}

fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) / alignment * alignment
}

// ---------------------------------------------------------------------------
// POD structs
// ---------------------------------------------------------------------------

/// On-disk BVH node: a fixed-size POD mirror of `NaniteBlob::BlobBVHNode`.
///
/// 96 bytes, 16-byte aligned. Offsets: `p_min` 0, `p_max` 16, `parent_bounding_sphere` 32,
/// `children` 48, `error_world` 64, `start` 72, `end` 76, `depth` 80, `node_status` 84,
/// `lod_level` 88, `index` 92.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlobBvhNode {
    pub p_min: [f32; 3],
    pub p_max: [f32; 3],
    /// `(center.xyz, radius)` used for the *parent* error term.
    pub parent_bounding_sphere: [f32; 4],
    /// Local (per-LOD) child node indices, `-1` terminated.
    pub children: [i32; 4],
    /// `(normalizedLodError, parentNormalizedError)`.
    pub error_world: [f32; 2],
    /// `[start, end)` into `SECTION_SORTED_CLUSTER_INDICES`.
    pub start: u32,
    pub end: u32,
    pub depth: i32,
    pub node_status: i32,
    pub lod_level: i32,
    /// Index within this LOD's flattened BVH.
    pub index: i32,
}

impl Default for BlobBvhNode {
    fn default() -> Self {
        Self {
            p_min: [f32::MAX; 3],
            p_max: [-f32::MAX; 3],
            parent_bounding_sphere: [0.0; 4],
            children: [-1; 4],
            error_world: [-f32::MAX; 2],
            start: 0,
            end: 0,
            depth: 0,
            node_status: node_status::INVALID,
            lod_level: -1,
            index: -1,
        }
    }
}

impl BlobBvhNode {
    pub const SIZE: usize = SIZE_BLOB_BVH_NODE as usize;

    fn encode(&self, out: &mut Vec<u8>) {
        write_vec3(out, self.p_min);
        out.extend_from_slice(&[0; 4]); // alignas(16) padding after a vec3
        write_vec3(out, self.p_max);
        out.extend_from_slice(&[0; 4]);
        write_vec4(out, self.parent_bounding_sphere);
        for c in self.children {
            out.extend_from_slice(&c.to_le_bytes());
        }
        write_f32(out, self.error_world[0]);
        write_f32(out, self.error_world[1]);
        out.extend_from_slice(&self.start.to_le_bytes());
        out.extend_from_slice(&self.end.to_le_bytes());
        out.extend_from_slice(&self.depth.to_le_bytes());
        out.extend_from_slice(&self.node_status.to_le_bytes());
        out.extend_from_slice(&self.lod_level.to_le_bytes());
        out.extend_from_slice(&self.index.to_le_bytes());
    }

    fn decode(b: &[u8]) -> Self {
        let mut node = Self::default();
        node.p_min = read_vec3(b, 0);
        node.p_max = read_vec3(b, 16);
        node.parent_bounding_sphere = read_vec4(b, 32);
        node.children = [
            i32::from_le_bytes(b[48..52].try_into().unwrap()),
            i32::from_le_bytes(b[52..56].try_into().unwrap()),
            i32::from_le_bytes(b[56..60].try_into().unwrap()),
            i32::from_le_bytes(b[60..64].try_into().unwrap()),
        ];
        node.error_world = [f32::from_le_bytes(b[64..68].try_into().unwrap()), f32::from_le_bytes(b[68..72].try_into().unwrap())];
        node.start = u32::from_le_bytes(b[72..76].try_into().unwrap());
        node.end = u32::from_le_bytes(b[76..80].try_into().unwrap());
        node.depth = i32::from_le_bytes(b[80..84].try_into().unwrap());
        node.node_status = i32::from_le_bytes(b[84..88].try_into().unwrap());
        node.lod_level = i32::from_le_bytes(b[88..92].try_into().unwrap());
        node.index = i32::from_le_bytes(b[92..96].try_into().unwrap());
        node
    }
}

/// GPU `ClusterInfo`, 48 bytes. Offsets: `p_min_world` 0, `p_max_world` 16,
/// `triangle_indices_start` 28, `triangle_indices_end` 32, `object_idx` 36.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterInfo {
    pub p_min_world: [f32; 3],
    pub p_max_world: [f32; 3],
    /// `[start, end)` into the LOD's cluster-sorted triangle index range.
    pub triangle_indices_start: u32,
    pub triangle_indices_end: u32,
    pub object_idx: u32,
}

impl Default for ClusterInfo {
    fn default() -> Self {
        Self {
            p_min_world: [f32::MAX; 3],
            p_max_world: [-f32::MAX; 3],
            triangle_indices_start: 0,
            triangle_indices_end: 0,
            object_idx: 0,
        }
    }
}

impl ClusterInfo {
    pub const SIZE: usize = SIZE_CLUSTER_INFO as usize;

    fn encode(&self, out: &mut Vec<u8>) {
        // `pMaxWorld` is `alignas(16)` but `glm::vec3` is only 4-byte aligned, so it starts at
        // 16 and ends at 28 - there is *no* padding between it and `triangleIndicesStart`.
        write_vec3(out, self.p_min_world);
        out.extend_from_slice(&[0; 4]);
        write_vec3(out, self.p_max_world);
        out.extend_from_slice(&self.triangle_indices_start.to_le_bytes());
        out.extend_from_slice(&self.triangle_indices_end.to_le_bytes());
        out.extend_from_slice(&self.object_idx.to_le_bytes());
        out.extend_from_slice(&[0; 8]); // trailing padding to alignof 16
    }

    fn decode(b: &[u8]) -> Self {
        Self {
            p_min_world: read_vec3(b, 0),
            p_max_world: read_vec3(b, 16),
            triangle_indices_start: u32::from_le_bytes(b[28..32].try_into().unwrap()),
            triangle_indices_end: u32::from_le_bytes(b[32..36].try_into().unwrap()),
            object_idx: u32::from_le_bytes(b[36..40].try_into().unwrap()),
        }
    }
}

/// GPU `ErrorInfo`, 48 bytes. Offsets: `error_world` 0, `center_r` 16, `center_rp` 32.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorInfo {
    /// `(lodError, parentError)` in world units, before projection.
    pub error_world: [f32; 2],
    /// `(clusterBoundingSphereCenter.xyz, worldRadius)`.
    pub center_r: [f32; 4],
    /// `(parentBoundingSphereCenter.xyz, parentBoundingRadius)`.
    pub center_rp: [f32; 4],
}

impl Default for ErrorInfo {
    fn default() -> Self {
        Self {
            error_world: [-f32::MAX; 2],
            center_r: [0.0; 4],
            center_rp: [0.0; 4],
        }
    }
}

impl ErrorInfo {
    pub const SIZE: usize = SIZE_ERROR_INFO as usize;

    fn encode(&self, out: &mut Vec<u8>) {
        write_f32(out, self.error_world[0]);
        write_f32(out, self.error_world[1]);
        out.extend_from_slice(&[0; 8]); // alignas(16) padding after a vec2
        write_vec4(out, self.center_r);
        write_vec4(out, self.center_rp);
    }

    fn decode(b: &[u8]) -> Self {
        Self {
            error_world: [f32::from_le_bytes(b[0..4].try_into().unwrap()), f32::from_le_bytes(b[4..8].try_into().unwrap())],
            center_r: read_vec4(b, 16),
            center_rp: read_vec4(b, 32),
        }
    }
}

/// GPU vertex, an exact mirror of `vkglTF::Vertex` (`base/VulkanglTFModel.h`). 112 bytes.
///
/// Offsets: `pos` 0, `normal` 16, `uv` 32, `color` 48, `joint0` 64, `weight0` 80, `tangent` 96.
/// The 4 bytes after `pos` and after `normal`, and the 8 bytes after `uv`, are `alignas` padding;
/// the C++ builder leaves them uninitialized (they hold garbage), and nothing ever reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// The C++ builder splats the LOD level into all four components; the viewer's
    /// "Visualize Clusters" mode 1 reads `joint0.xyz` as the LOD colour.
    pub joint0: [f32; 4],
    pub weight0: [f32; 4],
    pub tangent: [f32; 4],
}

impl Vertex {
    pub const SIZE: usize = SIZE_VERTEX as usize;

    fn encode(&self, out: &mut Vec<u8>) {
        write_vec3(out, self.pos);
        out.extend_from_slice(&[0; 4]);
        write_vec3(out, self.normal);
        out.extend_from_slice(&[0; 4]);
        write_f32(out, self.uv[0]);
        write_f32(out, self.uv[1]);
        out.extend_from_slice(&[0; 8]);
        write_vec4(out, self.color);
        write_vec4(out, self.joint0);
        write_vec4(out, self.weight0);
        write_vec4(out, self.tangent);
    }

    fn decode(b: &[u8]) -> Self {
        Self {
            pos: read_vec3(b, 0),
            normal: read_vec3(b, 16),
            uv: [f32::from_le_bytes(b[32..36].try_into().unwrap()), f32::from_le_bytes(b[36..40].try_into().unwrap())],
            color: read_vec4(b, 48),
            joint0: read_vec4(b, 64),
            weight0: read_vec4(b, 80),
            tangent: read_vec4(b, 96),
        }
    }
}

/// The GPU `BVHNodeInfo` is *not* stored in the blob - the viewer derives it from
/// [`BlobBvhNode`] plus the instance transform - but its size is recorded in the header, so the
/// constants above are checked against it.

fn write_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_vec3(out: &mut Vec<u8>, v: [f32; 3]) {
    for c in v {
        write_f32(out, c);
    }
}

fn write_vec4(out: &mut Vec<u8>, v: [f32; 4]) {
    for c in v {
        write_f32(out, c);
    }
}

fn read_f32(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn read_vec3(b: &[u8], at: usize) -> [f32; 3] {
    [read_f32(b, at), read_f32(b, at + 4), read_f32(b, at + 8)]
}

fn read_vec4(b: &[u8], at: usize) -> [f32; 4] {
    [read_f32(b, at), read_f32(b, at + 4), read_f32(b, at + 8), read_f32(b, at + 12)]
}

// ---------------------------------------------------------------------------
// Header and section table
// ---------------------------------------------------------------------------

/// 64 bytes. Offsets: `magic` 0, `version` 4, `builder_version` 8, *(pad)* 12, `source_hash` 16,
/// `config_hash` 24, `lod_nums` 32, `section_count` 36, `size_bvh_node_info` 40,
/// `size_cluster_info` 44, `size_error_info` 48, `size_vertex` 52, `size_blob_bvh_node` 56,
/// `pad` 60.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub magic: u32,
    pub version: u32,
    pub builder_version: u32,
    pub source_hash: u64,
    pub config_hash: u64,
    pub lod_nums: u32,
    pub section_count: u32,
    pub size_bvh_node_info: u32,
    pub size_cluster_info: u32,
    pub size_error_info: u32,
    pub size_vertex: u32,
    pub size_blob_bvh_node: u32,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            magic: NANITE_BLOB_MAGIC,
            version: NANITE_BLOB_VERSION,
            builder_version: NANITE_BUILDER_VERSION,
            source_hash: 0,
            config_hash: config_hash(),
            lod_nums: 0,
            section_count: 0,
            size_bvh_node_info: SIZE_BVH_NODE_INFO,
            size_cluster_info: SIZE_CLUSTER_INFO,
            size_error_info: SIZE_ERROR_INFO,
            size_vertex: SIZE_VERTEX,
            size_blob_bvh_node: SIZE_BLOB_BVH_NODE,
        }
    }
}

impl Header {
    pub const SIZE: usize = HEADER_SIZE;

    fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut out = Vec::with_capacity(HEADER_SIZE);
        out.extend_from_slice(&self.magic.to_le_bytes());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&self.builder_version.to_le_bytes());
        out.extend_from_slice(&[0; 4]); // padding so the u64 hashes land on 8 bytes
        out.extend_from_slice(&self.source_hash.to_le_bytes());
        out.extend_from_slice(&self.config_hash.to_le_bytes());
        out.extend_from_slice(&self.lod_nums.to_le_bytes());
        out.extend_from_slice(&self.section_count.to_le_bytes());
        out.extend_from_slice(&self.size_bvh_node_info.to_le_bytes());
        out.extend_from_slice(&self.size_cluster_info.to_le_bytes());
        out.extend_from_slice(&self.size_error_info.to_le_bytes());
        out.extend_from_slice(&self.size_vertex.to_le_bytes());
        out.extend_from_slice(&self.size_blob_bvh_node.to_le_bytes());
        out.extend_from_slice(&[0; 4]); // trailing pad
        debug_assert_eq!(out.len(), HEADER_SIZE);
        out.try_into().unwrap()
    }

    fn decode(b: &[u8]) -> Self {
        Self {
            magic: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            version: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            builder_version: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            source_hash: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            config_hash: u64::from_le_bytes(b[24..32].try_into().unwrap()),
            lod_nums: u32::from_le_bytes(b[32..36].try_into().unwrap()),
            section_count: u32::from_le_bytes(b[36..40].try_into().unwrap()),
            size_bvh_node_info: u32::from_le_bytes(b[40..44].try_into().unwrap()),
            size_cluster_info: u32::from_le_bytes(b[44..48].try_into().unwrap()),
            size_error_info: u32::from_le_bytes(b[48..52].try_into().unwrap()),
            size_vertex: u32::from_le_bytes(b[52..56].try_into().unwrap()),
            size_blob_bvh_node: u32::from_le_bytes(b[56..60].try_into().unwrap()),
        }
    }
}

/// 24 bytes: `key` u32, `count` u32, `offset` u64, `size` u64.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub key: u32,
    pub count: u32,
    pub offset: u64,
    pub size: u64,
}

impl Section {
    pub const SIZE: usize = SECTION_SIZE;

    fn encode(&self) -> [u8; SECTION_SIZE] {
        let mut out = Vec::with_capacity(SECTION_SIZE);
        out.extend_from_slice(&self.key.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.offset.to_le_bytes());
        out.extend_from_slice(&self.size.to_le_bytes());
        out.try_into().unwrap()
    }

    fn decode(b: &[u8]) -> Self {
        Self {
            key: u32::from_le_bytes(b[0..4].try_into().unwrap()),
            count: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            offset: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            size: u64::from_le_bytes(b[16..24].try_into().unwrap()),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("blob: cannot open {0}: {1}")]
    CannotOpen(String, std::io::Error),
    #[error("blob: cannot create {0}: {1}")]
    CannotCreate(String, std::io::Error),
    #[error("blob: truncated header: {0}")]
    TruncatedHeader(String),
    #[error("blob: bad magic: {0} (found 0x{1:08X})")]
    BadMagic(String, u32),
    #[error("blob: version mismatch (file {file}, expected {expected})")]
    VersionMismatch { file: u32, expected: u32 },
    #[error("blob: struct size mismatch - rebuild the cache (clusterInfo {found} != {expected})")]
    StructSizeMismatch { found: u32, expected: u32 },
    #[error("blob: builder config changed - rebuild the cache (file 0x{file:016X}, expected 0x{expected:016X})")]
    ConfigChanged { file: u64, expected: u64 },
    #[error("blob: implausible section count: {0}")]
    ImplausibleSectionCount(u32),
    #[error("blob: truncated section table")]
    TruncatedSectionTable,
    #[error("blob: failed reading {0}")]
    SectionReadFailed(&'static str),
    #[error("blob: BVH data missing")]
    BvhDataMissing,
    #[error("blob: clusterInfo / errorInfo count mismatch ({clusters} != {errors})")]
    CountMismatch { clusters: usize, errors: usize },
    #[error("blob: source mesh changed - rebuild the cache (file 0x{file:016X}, expected 0x{expected:016X})")]
    SourceChanged { file: u64, expected: u64 },
    #[error("blob: vertexStride is 0")]
    ZeroVertexStride,
    #[error("blob: {0}")]
    Other(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, BlobError>;

// ---------------------------------------------------------------------------
// The asset
// ---------------------------------------------------------------------------

/// Everything the viewer needs to upload a mesh without rebuilding it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asset {
    /// Where the blob came from. Written as `SECTION_SOURCE_PATH`, stored **absolute and
    /// canonical** by the builder so it cannot be re-resolved against a different cwd.
    pub source_path: String,
    pub source_hash: u64,
    pub lod_nums: u32,
    /// Byte size of one vertex element.
    pub vertex_stride: u32,
    /// Unique vertices, LOD0..LODn concatenated.
    pub vertices: Vec<Vertex>,
    /// Triangle vertex indices, cluster-sorted, already offset into `vertices`.
    pub indices: Vec<u32>,
    pub cluster_infos: Vec<ClusterInfo>,
    pub error_infos: Vec<ErrorInfo>,
    /// All LODs' flattened BVHs, concatenated. Child indices are *local* to each LOD.
    pub bvh_nodes: Vec<BlobBvhNode>,
    /// Leaf -> global cluster index.
    pub sorted_cluster_indices: Vec<u32>,
    /// Cluster count of each LOD.
    pub lod_cluster_nums: Vec<u32>,
    /// Local indices of the virtual root's children, one per LOD.
    pub bvh_roots: Vec<u32>,
}

impl Asset {
    pub fn cluster_count(&self) -> usize {
        self.cluster_infos.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }
}

/// A blob read without any validation, for diagnostics (`nanite-bake-rs --verify`).
#[derive(Clone, Debug)]
pub struct BlobInfo {
    pub path: String,
    pub file_size: u64,
    pub header: Header,
    pub sections: Vec<Section>,
}

impl BlobInfo {
    pub fn section(&self, key: u32) -> Option<&Section> {
        self.sections.iter().find(|s| s.key == key)
    }
}

/// Read the header and section table of any file, without validating versions or hashes.
pub fn inspect(path: &Path) -> Result<BlobInfo> {
    let mut file = File::open(path).map_err(|e| BlobError::CannotOpen(path.display().to_string(), e))?;
    let file_size = file.metadata()?.len();
    let mut raw = [0u8; HEADER_SIZE];
    if file.read_exact(&mut raw).is_err() {
        return Err(BlobError::TruncatedHeader(path.display().to_string()));
    }
    let header = Header::decode(&raw);
    let mut sections = Vec::new();
    for _ in 0..header.section_count {
        let mut raw = [0u8; SECTION_SIZE];
        if file.read_exact(&mut raw).is_err() {
            break;
        }
        sections.push(Section::decode(&raw));
    }
    Ok(BlobInfo {
        path: path.display().to_string(),
        file_size,
        header,
        sections,
    })
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Write `asset` to `path` with `source_hash` recorded in the header.
///
/// The section order is fixed (see [`section`]) and every payload is 256-byte aligned, which is
/// what makes a blob written here byte-comparable with one written by the C++ builder.
pub fn write(path: &Path, asset: &Asset, source_hash: u64) -> Result<()> {
    if asset.vertex_stride == 0 {
        return Err(BlobError::ZeroVertexStride);
    }
    if asset.cluster_infos.len() != asset.error_infos.len() {
        return Err(BlobError::CountMismatch {
            clusters: asset.cluster_infos.len(),
            errors: asset.error_infos.len(),
        });
    }

    // Payloads in the fixed section order. `SECTION_VERTICES` has elemSize 1 in the C++ writer, so
    // its `count` is the *byte* size, not the vertex count - do not "fix" that, the reader and the
    // C++ builder both rely on it.
    let mut vertex_bytes = Vec::with_capacity(asset.vertices.len() * Vertex::SIZE);
    for v in &asset.vertices {
        v.encode(&mut vertex_bytes);
    }
    let mut cluster_bytes = Vec::with_capacity(asset.cluster_infos.len() * ClusterInfo::SIZE);
    for c in &asset.cluster_infos {
        c.encode(&mut cluster_bytes);
    }
    let mut error_bytes = Vec::with_capacity(asset.error_infos.len() * ErrorInfo::SIZE);
    for e in &asset.error_infos {
        e.encode(&mut error_bytes);
    }
    let mut bvh_bytes = Vec::with_capacity(asset.bvh_nodes.len() * BlobBvhNode::SIZE);
    for n in &asset.bvh_nodes {
        n.encode(&mut bvh_bytes);
    }

    let payloads: [(u32, &[u8], u32); section::COUNT] = [
        (section::VERTICES, &vertex_bytes, vertex_bytes.len() as u32),
        (section::INDICES, u32_slice_to_bytes(&asset.indices), asset.indices.len() as u32),
        (section::CLUSTER_INFO, &cluster_bytes, asset.cluster_infos.len() as u32),
        (section::ERROR_INFO, &error_bytes, asset.error_infos.len() as u32),
        (section::BVH_NODES, &bvh_bytes, asset.bvh_nodes.len() as u32),
        (section::SORTED_CLUSTER_INDICES, u32_slice_to_bytes(&asset.sorted_cluster_indices), asset.sorted_cluster_indices.len() as u32),
        (section::LOD_CLUSTER_NUMS, u32_slice_to_bytes(&asset.lod_cluster_nums), asset.lod_cluster_nums.len() as u32),
        (section::BVH_ROOTS, u32_slice_to_bytes(&asset.bvh_roots), asset.bvh_roots.len() as u32),
        (section::SOURCE_PATH, asset.source_path.as_bytes(), asset.source_path.len() as u32),
    ];

    let mut sections = Vec::with_capacity(section::COUNT);
    let mut cursor = align_up((HEADER_SIZE + SECTION_SIZE * section::COUNT) as u64, PAYLOAD_ALIGNMENT);
    for &(key, data, count) in &payloads {
        let size = data.len() as u64;
        sections.push(Section { key, count, offset: cursor, size });
        cursor = align_up(cursor + size, PAYLOAD_ALIGNMENT);
    }

    let header = Header {
        source_hash,
        config_hash: config_hash(),
        lod_nums: asset.lod_nums,
        section_count: sections.len() as u32,
        size_vertex: asset.vertex_stride,
        ..Header::default()
    };

    let file = File::create(path).map_err(|e| BlobError::CannotCreate(path.display().to_string(), e))?;
    let mut out = BufWriter::new(file);
    out.write_all(&header.encode())?;
    for s in &sections {
        out.write_all(&s.encode())?;
    }
    // Pad up to the first payload, then pad between payloads so each lands on its recorded offset.
    let first_offset = sections.first().map_or(cursor, |s| s.offset);
    let mut pos = (HEADER_SIZE + sections.len() * SECTION_SIZE) as u64;
    while pos < first_offset {
        out.write_all(&[0])?;
        pos += 1;
    }
    for &(key, data, _) in &payloads {
        let sec = sections.iter().find(|s| s.key == key).expect("section written above");
        while pos < sec.offset {
            out.write_all(&[0])?;
            pos += 1;
        }
        if !data.is_empty() {
            out.write_all(data)?;
            pos += data.len() as u64;
        }
    }
    out.flush()?;
    Ok(())
}

fn u32_slice_to_bytes(values: &[u32]) -> &[u8] {
    // `u32` has align 4, so a byte view of the slice is well defined on every supported target.
    // The blob is little endian, matching x86-64/ARM LE - the same assumption the C++ `memcpy`
    // based writer makes.
    let ptr = values.as_ptr().cast::<u8>();
    // SAFETY: `values` is a valid, aligned, non-null slice of `u32`; `len * 4` bytes are
    // therefore all part of the same allocation and readable.
    unsafe { std::slice::from_raw_parts(ptr, values.len() * 4) }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Options for [`read`].
#[derive(Clone, Copy, Debug, Default)]
pub struct ReadOptions {
    /// Fail when the source mesh still exists but its bytes no longer hash to the recorded
    /// `sourceHash`. The viewer's blob-only path (`--model bunny.fcc`) uses this: there is no
    /// fallback, so a stale blob has to be reported instead of rendering stale geometry.
    pub strict_source_hash: bool,
}

/// Read a blob, validating it exactly like `NaniteBlob::read`.
///
/// A `source_path` already set on the returned asset is kept (that file is what a strict hash
/// check re-reads); otherwise it is filled from `SECTION_SOURCE_PATH`, which is how a standalone
/// `.fcc` knows where it came from.
pub fn read(path: &Path, options: ReadOptions) -> Result<Asset> {
    // The whole blob is slurped: it is at most a few tens of MB, and it keeps the section readers
    // immutable, which the C++ `ifstream` version does not have to worry about.
    let file = std::fs::read(path).map_err(|e| BlobError::CannotOpen(path.display().to_string(), e))?;
    if file.len() < HEADER_SIZE {
        return Err(BlobError::TruncatedHeader(path.display().to_string()));
    }
    let header = Header::decode(&file[..HEADER_SIZE]);
    if header.magic != NANITE_BLOB_MAGIC {
        return Err(BlobError::BadMagic(path.display().to_string(), header.magic));
    }
    if header.version != NANITE_BLOB_VERSION {
        return Err(BlobError::VersionMismatch { file: header.version, expected: NANITE_BLOB_VERSION });
    }
    if header.size_cluster_info != SIZE_CLUSTER_INFO {
        return Err(BlobError::StructSizeMismatch { found: header.size_cluster_info, expected: SIZE_CLUSTER_INFO });
    }
    if header.size_error_info != SIZE_ERROR_INFO {
        return Err(BlobError::StructSizeMismatch { found: header.size_error_info, expected: SIZE_ERROR_INFO });
    }
    if header.size_blob_bvh_node != SIZE_BLOB_BVH_NODE {
        return Err(BlobError::StructSizeMismatch { found: header.size_blob_bvh_node, expected: SIZE_BLOB_BVH_NODE });
    }
    if header.config_hash != config_hash() {
        return Err(BlobError::ConfigChanged { file: header.config_hash, expected: config_hash() });
    }
    if header.section_count > (section::COUNT as u32) * 4 {
        return Err(BlobError::ImplausibleSectionCount(header.section_count));
    }

    let table_end = HEADER_SIZE + header.section_count as usize * SECTION_SIZE;
    if file.len() < table_end {
        return Err(BlobError::TruncatedSectionTable);
    }
    let sections: Vec<Section> = (0..header.section_count as usize)
        .map(|i| Section::decode(&file[HEADER_SIZE + i * SECTION_SIZE..][..SECTION_SIZE]))
        .collect();

    let find = |key: u32| sections.iter().find(|s| s.key == key);
    let slice = |key: u32, what: &'static str| -> Result<&[u8]> {
        let Some(sec) = find(key) else {
            return Ok(&[]);
        };
        let start = usize::try_from(sec.offset).map_err(|_| BlobError::SectionReadFailed(what))?;
        let end = start
            .checked_add(usize::try_from(sec.size).map_err(|_| BlobError::SectionReadFailed(what))?)
            .ok_or(BlobError::SectionReadFailed(what))?;
        file.get(start..end).ok_or(BlobError::SectionReadFailed(what))
    };
    let u32s = |key: u32, what: &'static str| -> Result<Vec<u32>> {
        Ok(slice(key, what)?
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect())
    };
    let pod = |key: u32, size: usize, what: &'static str| -> Result<&[u8]> {
        let raw = slice(key, what)?;
        if raw.len() % size != 0 {
            return Err(BlobError::SectionReadFailed(what));
        }
        Ok(raw)
    };

    let mut asset = Asset::default();
    asset.lod_nums = header.lod_nums;
    asset.source_hash = header.source_hash;
    asset.vertex_stride = header.size_vertex;

    let vertices = pod(section::VERTICES, Vertex::SIZE, "vertices")?;
    asset.vertices = vertices.chunks_exact(Vertex::SIZE).map(Vertex::decode).collect();

    asset.indices = u32s(section::INDICES, "indices")?;

    asset.cluster_infos = pod(section::CLUSTER_INFO, ClusterInfo::SIZE, "clusterInfo")?
        .chunks_exact(ClusterInfo::SIZE)
        .map(ClusterInfo::decode)
        .collect();

    asset.error_infos = pod(section::ERROR_INFO, ErrorInfo::SIZE, "errorInfo")?
        .chunks_exact(ErrorInfo::SIZE)
        .map(ErrorInfo::decode)
        .collect();

    asset.bvh_nodes = pod(section::BVH_NODES, BlobBvhNode::SIZE, "bvhNodes")?
        .chunks_exact(BlobBvhNode::SIZE)
        .map(BlobBvhNode::decode)
        .collect();

    asset.sorted_cluster_indices = u32s(section::SORTED_CLUSTER_INDICES, "sortedClusterIndices")?;
    asset.lod_cluster_nums = u32s(section::LOD_CLUSTER_NUMS, "lodClusterNums")?;
    asset.bvh_roots = u32s(section::BVH_ROOTS, "bvhRoots")?;

    // Informational, so a blob written by an older builder is still perfectly usable - it just
    // cannot re-hash its source.
    let source_raw = slice(section::SOURCE_PATH, "sourcePath")?;
    if asset.source_path.is_empty() && !source_raw.is_empty() {
        asset.source_path = String::from_utf8_lossy(&source_raw).into_owned();
    }

    if asset.bvh_nodes.is_empty() || asset.bvh_roots.is_empty() {
        return Err(BlobError::BvhDataMissing);
    }
    if asset.cluster_infos.len() != asset.error_infos.len() {
        return Err(BlobError::CountMismatch {
            clusters: asset.cluster_infos.len(),
            errors: asset.error_infos.len(),
        });
    }

    // Checked last on purpose: `source_path` may only have become known through
    // SECTION_SOURCE_PATH. A path that does not exist hashes to 0 and is skipped, so a standalone
    // .fcc whose mesh has been deleted still loads.
    if options.strict_source_hash {
        let current = fnv1a_file(Path::new(&asset.source_path));
        if current != 0 && current != header.source_hash {
            return Err(BlobError::SourceChanged { file: current, expected: header.source_hash });
        }
    }
    Ok(asset)
}

/// True if a blob file is present at `path`. The contents are validated by [`read`].
pub fn exists(path: &Path) -> bool {
    File::open(path).is_ok()
}

/// `<model>.fcc` next to `model_path`. Idempotent for a `.fcc` input, which is what lets a viewer
/// hand a picked `.fcc` straight to it. Mirrors `NaniteMesh::blobPathFor`.
pub fn blob_path_for(model_path: &Path) -> std::path::PathBuf {
    if is_blob_path(model_path) {
        return model_path.to_path_buf();
    }
    model_path.with_extension(&NANITE_BLOB_EXTENSION[1..])
}

/// Case-insensitive `.fcc` test. Mirrors `NaniteMesh::isBlobPath`.
pub fn is_blob_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(&NANITE_BLOB_EXTENSION[1..]))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests;
