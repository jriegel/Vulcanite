//! The shared bake pipeline: glTF in, `NaniteBlob::Asset` out.
//!
//! Extracted from `main.rs` so the viewer can load a glTF through the *same* code path the CLI
//! bakes with. That is the whole point of the split: `--model bunny.gltf` and
//! `--model bunny_rs.fcc` must produce the same geometry, or "the viewer can also read glTF" is
//! untestable.

use std::path::Path;

use anyhow::{Context, Result};
use nanite_blob::Asset;

use crate::{bvh, cluster, emit, gltf_load, mesh};

/// Options for one bake. Mirrors the CLI flags that change the output.
#[derive(Debug, Clone, Copy)]
pub struct BakeOptions {
	/// Skip `check_asset`, i.e. the pre-write gate the viewer's asserts amount to.
	pub skip_check: bool,
}

impl Default for BakeOptions {
	fn default() -> Self {
		Self { skip_check: false }
	}
}

/// What a bake produced, for logging / the UI.
#[derive(Debug, Clone)]
pub struct BakeReport {
	pub node_name: String,
	pub triangles: usize,
	pub clusters: usize,
	pub cluster_groups: usize,
	pub bvh_nodes: usize,
	pub lods: u32,
}

/// Build the in-memory blob for a glTF, without writing it anywhere.
///
/// The glTF is loaded through the same flag set the C++ builder uses (`PreTransformVertices |
/// PreMultiplyVertexColors | FlipY`): `load_first_mesh` already applies the node matrix and flips
/// Y, so the builder consumes exactly those vertices.
pub fn bake_asset(path: &Path, options: BakeOptions) -> Result<(Asset, BakeReport)> {
	let loaded = gltf_load::load_first_mesh(path)
		.with_context(|| format!("loading {}", path.display()))?;
	tracing::info!(
		"loaded node {:?}: {} vertices, {} indices",
		loaded.node_name,
		loaded.vertices.len(),
		loaded.indices.len()
	);

	let mut lod = mesh::LodMesh::new(gltf_load::to_triangle_mesh(&loaded));
	cluster::build_clusters(&mut lod);
	cluster::build_cluster_groups(&mut lod);
	if let Err(problem) = cluster::check_partition(&lod) {
		anyhow::bail!("partition invalid: {problem}");
	}
	tracing::info!(
		"{} triangles -> {} clusters in {} groups",
		lod.mesh.face_count(),
		lod.clusters.len(),
		lod.cluster_groups.len()
	);

	bvh::build_bvh(&mut lod);
	let bvh_nodes = lod.bvh.as_ref().map(|b| b.nodes.len()).unwrap_or(0);
	tracing::info!("BVH: {bvh_nodes} nodes");

	let asset = emit::build_asset(&loaded, &lod, path);
	if !options.skip_check {
		if let Err(problem) = emit::check_asset(&asset) {
			anyhow::bail!("blob would not load: {problem}");
		}
	}

	let report = BakeReport {
		node_name: loaded.node_name.clone(),
		triangles: lod.mesh.face_count(),
		clusters: lod.clusters.len(),
		cluster_groups: lod.cluster_groups.len(),
		bvh_nodes,
		lods: asset.lod_nums,
	};
	Ok((asset, report))
}
