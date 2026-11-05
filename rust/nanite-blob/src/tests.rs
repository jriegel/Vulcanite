//! Tests for the `.fcc` format contract.
//!
//! The struct-level tests pin every size and offset by hand. The file-level tests run against the
//! blob baked by the C++ builder (`assets/models/bunny.fcc`) and are skipped when that file is
//! absent, so the suite still passes in a checkout without generated assets.

use super::*;
use std::path::{Path, PathBuf};

/// `assets/models/bunny.fcc`, found relative to this crate.
fn repo_blob() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models/bunny.fcc")
}

// ---------------------------------------------------------------------------
// Struct layout
// ---------------------------------------------------------------------------

#[test]
fn struct_sizes_match_the_c_abi() {
    assert_eq!(BlobBvhNode::SIZE, 96);
    assert_eq!(ClusterInfo::SIZE, 48);
    assert_eq!(ErrorInfo::SIZE, 48);
    assert_eq!(Vertex::SIZE, 112);
    assert_eq!(Header::SIZE, 64);
    assert_eq!(Section::SIZE, 24);
}

#[test]
fn header_offsets() {
    let header = Header {
        magic: 0x1122_3344,
        version: 0x5566_7788,
        builder_version: 0x99AA_BBCC,
        source_hash: 0x0102_0304_0506_0708,
        config_hash: 0x1112_1314_1516_1718,
        lod_nums: 6,
        section_count: 9,
        size_bvh_node_info: 112,
        size_cluster_info: 48,
        size_error_info: 48,
        size_vertex: 112,
        size_blob_bvh_node: 96,
    };
    let b = header.encode();
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 0x1122_3344);
    assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 0x5566_7788);
    assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 0x99AA_BBCC);
    // The 4 bytes at 12 are padding inserted by the two u64 members.
    assert_eq!(&b[12..16], &[0; 4]);
    assert_eq!(u64::from_le_bytes(b[16..24].try_into().unwrap()), 0x0102_0304_0506_0708);
    assert_eq!(u64::from_le_bytes(b[24..32].try_into().unwrap()), 0x1112_1314_1516_1718);
    assert_eq!(u32::from_le_bytes(b[32..36].try_into().unwrap()), 6);
    assert_eq!(u32::from_le_bytes(b[36..40].try_into().unwrap()), 9);
    assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 112);
    assert_eq!(u32::from_le_bytes(b[44..48].try_into().unwrap()), 48);
    assert_eq!(u32::from_le_bytes(b[48..52].try_into().unwrap()), 48);
    assert_eq!(u32::from_le_bytes(b[52..56].try_into().unwrap()), 112);
    assert_eq!(u32::from_le_bytes(b[56..60].try_into().unwrap()), 96);
    assert_eq!(Header::decode(&b), header);
}

#[test]
fn blob_bvh_node_offsets() {
    let node = BlobBvhNode {
        p_min: [1.0, 2.0, 3.0],
        p_max: [4.0, 5.0, 6.0],
        parent_bounding_sphere: [7.0, 8.0, 9.0, 10.0],
        children: [11, 12, -1, -1],
        error_world: [13.0, 14.0],
        start: 15,
        end: 16,
        depth: 17,
        node_status: node_status::LEAF,
        lod_level: 18,
        index: 19,
    };
    let mut b = Vec::new();
    node.encode(&mut b);
    assert_eq!(b.len(), BlobBvhNode::SIZE);
    assert_eq!(read_vec3(&b, 0), [1.0, 2.0, 3.0]);
    assert_eq!(read_vec3(&b, 16), [4.0, 5.0, 6.0]);
    assert_eq!(read_vec4(&b, 32), [7.0, 8.0, 9.0, 10.0]);
    assert_eq!(i32::from_le_bytes(b[48..52].try_into().unwrap()), 11);
    assert_eq!(i32::from_le_bytes(b[60..64].try_into().unwrap()), -1);
    assert_eq!(read_f32(&b, 64), 13.0);
    assert_eq!(read_f32(&b, 68), 14.0);
    assert_eq!(u32::from_le_bytes(b[72..76].try_into().unwrap()), 15);
    assert_eq!(u32::from_le_bytes(b[76..80].try_into().unwrap()), 16);
    assert_eq!(i32::from_le_bytes(b[80..84].try_into().unwrap()), 17);
    assert_eq!(i32::from_le_bytes(b[84..88].try_into().unwrap()), node_status::LEAF);
    assert_eq!(i32::from_le_bytes(b[88..92].try_into().unwrap()), 18);
    assert_eq!(i32::from_le_bytes(b[92..96].try_into().unwrap()), 19);
    assert_eq!(BlobBvhNode::decode(&b), node);
}

#[test]
fn cluster_info_offsets() {
    // The C++ static_asserts put triangleIndicesStart/End/objectIdx at 28/32/36, *not* 32/36/40.
    let cluster = ClusterInfo {
        p_min_world: [1.0, 2.0, 3.0],
        p_max_world: [4.0, 5.0, 6.0],
        triangle_indices_start: 7,
        triangle_indices_end: 8,
        object_idx: 9,
    };
    let mut b = Vec::new();
    cluster.encode(&mut b);
    assert_eq!(b.len(), ClusterInfo::SIZE);
    assert_eq!(read_vec3(&b, 0), [1.0, 2.0, 3.0]);
    assert_eq!(read_vec3(&b, 16), [4.0, 5.0, 6.0]);
    assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 7);
    assert_eq!(u32::from_le_bytes(b[32..36].try_into().unwrap()), 8);
    assert_eq!(u32::from_le_bytes(b[36..40].try_into().unwrap()), 9);
    assert_eq!(ClusterInfo::decode(&b), cluster);
}

#[test]
fn error_info_offsets() {
    let error = ErrorInfo {
        error_world: [1.0, 2.0],
        center_r: [3.0, 4.0, 5.0, 6.0],
        center_rp: [7.0, 8.0, 9.0, 10.0],
    };
    let mut b = Vec::new();
    error.encode(&mut b);
    assert_eq!(b.len(), ErrorInfo::SIZE);
    assert_eq!(read_f32(&b, 0), 1.0);
    assert_eq!(read_f32(&b, 4), 2.0);
    assert_eq!(read_vec4(&b, 16), [3.0, 4.0, 5.0, 6.0]);
    assert_eq!(read_vec4(&b, 32), [7.0, 8.0, 9.0, 10.0]);
    assert_eq!(ErrorInfo::decode(&b), error);
}

#[test]
fn vertex_offsets() {
    // Verified empirically against bunny.fcc and against the std430 mirror in
    // shaders/glsl/pbrtexture/shading.frag: pos 0, normal 16, uv 32, color 48, joint0 64,
    // weight0 80, tangent 96.
    let vertex = Vertex {
        pos: [1.0, 2.0, 3.0],
        normal: [4.0, 5.0, 6.0],
        uv: [7.0, 8.0],
        color: [9.0, 10.0, 11.0, 12.0],
        joint0: [13.0, 13.0, 13.0, 13.0],
        weight0: [14.0, 14.0, 14.0, 14.0],
        tangent: [15.0, 16.0, 17.0, 18.0],
    };
    let mut b = Vec::new();
    vertex.encode(&mut b);
    assert_eq!(b.len(), Vertex::SIZE);
    assert_eq!(read_vec3(&b, 0), [1.0, 2.0, 3.0]);
    assert_eq!(read_vec3(&b, 16), [4.0, 5.0, 6.0]);
    assert_eq!(read_f32(&b, 32), 7.0);
    assert_eq!(read_f32(&b, 36), 8.0);
    assert_eq!(read_vec4(&b, 48), [9.0, 10.0, 11.0, 12.0]);
    assert_eq!(read_vec4(&b, 64), [13.0; 4]);
    assert_eq!(read_vec4(&b, 80), [14.0; 4]);
    assert_eq!(read_vec4(&b, 96), [15.0, 16.0, 17.0, 18.0]);
    assert_eq!(Vertex::decode(&b), vertex);
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

#[test]
fn config_hash_matches_the_c_builder() {
    // Measured from assets/models/bunny.fcc.
    assert_eq!(config_hash(), 14_829_784_758_405_755_283);
}

#[test]
fn fnv1a_matches_the_reference_vectors() {
    // Standard FNV-1a 64 bit test vectors.
    assert_eq!(fnv1a(b"", FNV1A_OFFSET_BASIS), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a(b"a", FNV1A_OFFSET_BASIS), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a(b"foobar", FNV1A_OFFSET_BASIS), 0x8594_4171_f739_67e8);
}

#[test]
fn fnv1a_file_of_a_missing_file_is_zero() {
    // 0 means "unknown" and makes `read` skip the strict check, which is what keeps a shipped
    // blob usable after its source mesh has been deleted.
    assert_eq!(fnv1a_file(Path::new("Z:/definitely/not/here.gltf")), 0);
}

#[test]
fn fnv1a_file_matches_fnv1a_over_the_whole_file() {
    let path = std::env::temp_dir().join("vulcanite_fnv_test.bin");
    let data: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(7) % 251) as u8).collect();
    std::fs::write(&path, &data).unwrap();
    assert_eq!(fnv1a_file(&path), fnv1a(&data, FNV1A_OFFSET_BASIS));
    let _ = std::fs::remove_file(path);
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

#[test]
fn blob_path_helpers() {
    assert_eq!(blob_path_for(Path::new("a/b/bunny.gltf")), PathBuf::from("a/b/bunny.fcc"));
    // Idempotent for a .fcc input, which is what lets a viewer hand a picked blob straight back.
    assert_eq!(blob_path_for(Path::new("a/b/bunny.fcc")), PathBuf::from("a/b/bunny.fcc"));
    assert!(is_blob_path(Path::new("x/bunny.fcc")));
    assert!(is_blob_path(Path::new("x/bunny.FCC")));
    assert!(!is_blob_path(Path::new("x/bunny.gltf")));
    assert!(!is_blob_path(Path::new("x/bunny")));
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------

fn synthetic_asset() -> Asset {
    let mut asset = Asset {
        source_path: "C:/somewhere/bunny.gltf".to_string(),
        source_hash: 0xDEAD_BEEF_0000_0001,
        lod_nums: 2,
        vertex_stride: Vertex::SIZE as u32,
        ..Asset::default()
    };
    for lod in 0..2u32 {
        for i in 0..4u32 {
            asset.vertices.push(Vertex {
                pos: [i as f32, lod as f32, 3.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.25, 0.75],
                color: [1.0; 4],
                joint0: [lod as f32; 4],
                weight0: [0.0; 4],
                tangent: [1.0, 0.0, 0.0, 1.0],
            });
        }
    }
    asset.indices = (0..24).collect();
    for i in 0..4u32 {
        asset.cluster_infos.push(ClusterInfo {
            p_min_world: [-1.0, -2.0, -3.0],
            p_max_world: [1.0, 2.0, 3.0],
            triangle_indices_start: i * 6,
            triangle_indices_end: i * 6 + 6,
            object_idx: 0,
        });
        asset.error_infos.push(ErrorInfo {
            error_world: [0.0, 1e5],
            center_r: [0.0, 0.0, 0.0, 0.5],
            center_rp: [0.0, 0.0, 0.0, 0.75],
        });
    }
    // Virtual root at index 0, one leaf LOD root at 1.
    asset.bvh_nodes = vec![
        BlobBvhNode {
            children: [1, -1, -1, -1],
            depth: 0,
            node_status: node_status::VIRTUAL_NODE,
            index: 0,
            ..BlobBvhNode::default()
        },
        BlobBvhNode {
            p_min: [-1.0, -2.0, -3.0],
            p_max: [1.0, 2.0, 3.0],
            parent_bounding_sphere: [0.0, 0.0, 0.0, 0.75],
            children: [-1; 4],
            error_world: [0.0, 1e5],
            start: 0,
            end: 4,
            depth: 0,
            node_status: node_status::LEAF,
            lod_level: 0,
            index: 1,
        },
    ];
    asset.sorted_cluster_indices = vec![0, 1, 2, 3];
    asset.lod_cluster_nums = vec![4];
    asset.bvh_roots = vec![1];
    asset
}

#[test]
fn write_then_read_round_trips() {
    let path = std::env::temp_dir().join("vulcanite_blob_roundtrip.fcc");
    let asset = synthetic_asset();
    write(&path, &asset, asset.source_hash).unwrap();
    let back = read(&path, ReadOptions::default()).unwrap();
    assert_eq!(back, asset);
    let _ = std::fs::remove_file(path);
}

#[test]
fn payloads_are_256_byte_aligned_and_ordered() {
    let path = std::env::temp_dir().join("vulcanite_blob_align.fcc");
    write(&path, &synthetic_asset(), 1).unwrap();
    let info = inspect(&path).unwrap();
    assert_eq!(info.header.section_count as usize, section::COUNT);
    let expected_keys = [
        section::VERTICES,
        section::INDICES,
        section::CLUSTER_INFO,
        section::ERROR_INFO,
        section::BVH_NODES,
        section::SORTED_CLUSTER_INDICES,
        section::LOD_CLUSTER_NUMS,
        section::BVH_ROOTS,
        section::SOURCE_PATH,
    ];
    for (i, s) in info.sections.iter().enumerate() {
        assert_eq!(s.key, expected_keys[i]);
        assert_eq!(s.offset % PAYLOAD_ALIGNMENT, 0, "section {} is not aligned", s.key);
        if i > 0 {
            assert!(s.offset >= info.sections[i - 1].offset + info.sections[i - 1].size);
        }
    }
    // alignUp(64 + 24 * 9, 256) == 512, the same first payload offset the C++ writer produces.
    assert_eq!(info.sections[0].offset, 512);
    let _ = std::fs::remove_file(path);
}

#[test]
fn stale_blob_is_rejected() {
    let path = std::env::temp_dir().join("vulcanite_blob_stale.fcc");
    write(&path, &synthetic_asset(), 1).unwrap();
    let bytes = std::fs::read(&path).unwrap();

    // (header offset, replacement value, expected error variant). Each case is matched by
    // discriminant - the payloads carry machine specific detail which is not worth comparing.
    let cases: Vec<(usize, u32, &str, fn(&BlobError) -> bool)> = vec![
        (0, NANITE_BLOB_MAGIC ^ 0xFF, "BadMagic", |e| matches!(e, BlobError::BadMagic(_, _))),
        (
            4,
            NANITE_BLOB_VERSION + 1,
            "VersionMismatch",
            |e| matches!(e, BlobError::VersionMismatch { .. }),
        ),
        (
            44,
            40,
            "StructSizeMismatch(clusterInfo)",
            |e| matches!(e, BlobError::StructSizeMismatch { found: 40, expected: 48 }),
        ),
        (
            48,
            40,
            "StructSizeMismatch(errorInfo)",
            |e| matches!(e, BlobError::StructSizeMismatch { found: 40, expected: 48 }),
        ),
        (
            56,
            90,
            "StructSizeMismatch(bvhNode)",
            |e| matches!(e, BlobError::StructSizeMismatch { found: 90, expected: 96 }),
        ),
        (
            36,
            999,
            "ImplausibleSectionCount",
            |e| matches!(e, BlobError::ImplausibleSectionCount(999)),
        ),
    ];
    for (offset, value, name, is_expected) in cases {
        let mut broken = bytes.clone();
        broken[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        std::fs::write(&path, &broken).unwrap();
        let err = read(&path, ReadOptions::default()).unwrap_err();
        assert!(
            is_expected(&err),
            "at byte {offset}: expected {name}, got {err:?}"
        );
    }

    // A corrupted config hash has to be reported as a builder change.
    let mut broken = bytes.clone();
    broken[24..32].copy_from_slice(&(config_hash() ^ 1).to_le_bytes());
    std::fs::write(&path, &broken).unwrap();
    assert!(matches!(
        read(&path, ReadOptions::default()).unwrap_err(),
        BlobError::ConfigChanged { .. }
    ));
    let _ = std::fs::remove_file(path);
}

#[test]
fn strict_source_hash_rejects_a_changed_mesh() {
    let dir = std::env::temp_dir().join("vulcanite_blob_strict");
    std::fs::create_dir_all(&dir).unwrap();
    let mesh = dir.join("mesh.gltf");
    let blob = dir.join("mesh.fcc");
    std::fs::write(&mesh, b"mesh bytes v1").unwrap();
    let hash = fnv1a_file(&mesh);
    assert_ne!(hash, 0);

    let mut asset = synthetic_asset();
    asset.source_path = mesh.display().to_string();
    write(&blob, &asset, hash).unwrap();

    // Unchanged source: both modes accept it.
    read(&blob, ReadOptions::default()).unwrap();
    read(&blob, ReadOptions { strict_source_hash: true }).unwrap();

    // Changed source: only the strict mode rejects it.
    std::fs::write(&mesh, b"mesh bytes v2").unwrap();
    read(&blob, ReadOptions::default()).unwrap();
    assert!(matches!(
        read(&blob, ReadOptions { strict_source_hash: true }).unwrap_err(),
        BlobError::SourceChanged { .. }
    ));

    // Deleted source: hashes to 0, which is skipped on purpose so a shipped blob still loads.
    std::fs::remove_file(&mesh).unwrap();
    read(&blob, ReadOptions { strict_source_hash: true }).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// The blob baked by the C++ builder
// ---------------------------------------------------------------------------

/// The values measured from `assets/models/bunny.fcc`. They are the acceptance target for any
/// other implementation of this format.
#[test]
fn reads_the_cpp_baked_blob() {
    let path = repo_blob();
    if !path.is_file() {
        eprintln!("skipping: {} does not exist", path.display());
        return;
    }
    let info = inspect(&path).unwrap();
    assert_eq!(info.header.magic, NANITE_BLOB_MAGIC);
    assert_eq!(info.header.version, NANITE_BLOB_VERSION);
    assert_eq!(info.header.builder_version, NANITE_BUILDER_VERSION);
    assert_eq!(info.header.source_hash, 5_547_983_326_492_162_387);
    assert_eq!(info.header.config_hash, config_hash());
    assert_eq!(info.header.lod_nums, 6);
    assert_eq!(info.header.section_count, 9);
    assert_eq!(info.header.size_bvh_node_info, 112);
    assert_eq!(info.header.size_cluster_info, 48);
    assert_eq!(info.header.size_error_info, 48);
    assert_eq!(info.header.size_vertex, 112);
    assert_eq!(info.header.size_blob_bvh_node, 96);

    let expect = |key: u32, count: u32, size: u64, offset: u64| {
        let s = info.section(key).unwrap_or_else(|| panic!("section {key} missing"));
        assert_eq!(s.count, count, "count of section {key}");
        assert_eq!(s.size, size, "size of section {key}");
        assert_eq!(s.offset, offset, "offset of section {key}");
    };
    // SECTION_VERTICES has elemSize 1, so its count is the *byte* size.
    expect(section::VERTICES, 16_092_720, 16_092_720, 512);
    expect(section::INDICES, 849_330, 3_397_320, 16_093_440);
    expect(section::CLUSTER_INFO, 5_212, 250_176, 19_490_816);
    expect(section::ERROR_INFO, 5_212, 250_176, 19_741_184);
    expect(section::BVH_NODES, 573, 55_008, 19_991_552);
    expect(section::SORTED_CLUSTER_INDICES, 5_212, 20_848, 20_046_592);
    expect(section::LOD_CLUSTER_NUMS, 6, 24, 20_067_584);
    expect(section::BVH_ROOTS, 6, 24, 20_067_840);
    expect(section::SOURCE_PATH, 48, 48, 20_068_096);

    let asset = read(
        &path,
        ReadOptions { strict_source_hash: true },
    )
    .expect("the C++ blob must load in strict mode - its source mesh is present");
    assert_eq!(asset.lod_nums, 6);
    assert_eq!(asset.vertex_count(), 143_685);
    assert_eq!(asset.indices.len(), 849_330);
    assert_eq!(asset.cluster_count(), 5_212);
    assert_eq!(asset.error_infos.len(), 5_212);
    assert_eq!(asset.bvh_nodes.len(), 573);
    assert_eq!(asset.sorted_cluster_indices.len(), 5_212);
    assert_eq!(asset.lod_cluster_nums, vec![2573, 1346, 694, 342, 172, 85]);
    assert_eq!(asset.bvh_roots, vec![1, 2, 3, 4, 5, 6]);
    assert!(asset.source_path.to_lowercase().ends_with("bunny.gltf"));

    // Node 0 is the builder's virtual root: an empty AABB, four of its six children inline.
    let root = asset.bvh_nodes[0];
    assert_eq!(root.node_status, node_status::VIRTUAL_NODE);
    assert_eq!(root.p_min, [f32::MAX; 3]);
    assert_eq!(root.p_max, [-f32::MAX; 3]);
    assert_eq!(root.children, [1, 2, 3, 4]);
    assert_eq!(root.index, 0);
    assert_eq!(root.lod_level, -1);

    // The per-LOD roots are the virtual root's children, listed in SECTION_BVH_ROOTS because
    // there are more than fit in the 4-wide `children` field.
    for (lod, &node_index) in asset.bvh_roots.iter().enumerate() {
        let node = asset.bvh_nodes[node_index as usize];
        assert_eq!(node.depth, 0);
        assert_eq!(node.lod_level, lod as i32);
        assert_eq!(node.node_status, node_status::NODE);
    }

    // The LOD level splatted into `joint0` is what the viewer's "Visualize Clusters" mode 1
    // colours with; it also pins the corrected Vertex offsets.
    let lod_starts = [0usize, 72_378, 108_678, 126_969, 136_278, 141_100];
    for (lod, &start) in lod_starts.iter().enumerate() {
        assert_eq!(asset.vertices[start].joint0[0], lod as f32, "vertex at {start}");
    }
    assert_eq!(asset.vertices.len(), 143_685);

    // Triangle ranges are contiguous per LOD and cover every index exactly once.
    assert_eq!(asset.cluster_infos[0].triangle_indices_start, 0);
    assert_eq!(asset.cluster_infos[0].triangle_indices_end, 56);
    assert_eq!(asset.cluster_infos[1].triangle_indices_start, 56);
    assert_eq!(asset.cluster_infos[5211].triangle_indices_end, 283_110);
    assert_eq!(
        asset.cluster_infos[5211].triangle_indices_end as usize * 3,
        asset.indices.len()
    );

    // The last LOD's clusters carry the "never culled" parent error the shaders need.
    let last_cluster = asset.error_infos[5211];
    assert_eq!(last_cluster.error_world[1], 1e5);
    assert!(last_cluster.center_rp[3] > last_cluster.center_r[3]);

    // Every leaf interval lands inside SECTION_SORTED_CLUSTER_INDICES.
    for node in &asset.bvh_nodes {
        if node.node_status == node_status::LEAF {
            assert!((node.end as usize) <= asset.sorted_cluster_indices.len());
            assert!(node.start <= node.end);
        }
    }
}

/// Re-writing the C++ blob with this crate and reading it back must reproduce every decoded value,
/// and must place every section at the same offset with the same size. (Not a byte-for-byte
/// compare: the C++ builder leaves `alignas` padding uninitialized, so those bytes are garbage.)
#[test]
fn rewriting_the_cpp_blob_is_lossless() {
    let source = repo_blob();
    if !source.is_file() {
        eprintln!("skipping: {} does not exist", source.display());
        return;
    }
    let asset = read(&source, ReadOptions::default()).unwrap();
    let original = inspect(&source).unwrap();

    let round = source.with_file_name("vulcanite_roundtrip.fcc");
    write(&round, &asset, asset.source_hash).unwrap();
    let back = read(&round, ReadOptions::default()).unwrap();
    let written = inspect(&round).unwrap();

    assert_eq!(back, asset);
    assert_eq!(written.file_size, original.file_size);
    assert_eq!(written.sections, original.sections);
    assert_eq!(written.header, original.header);
    let _ = std::fs::remove_file(round);
}
