//! `nanite-bake-rs` - the Rust front end of the asset pipeline.
//!
//! Reads a glTF and writes a `<model>.fcc` blob that the C++ viewer (`pbrtexture.exe`) loads
//! unmodified. The CLI mirrors `tools/nanite_bake` on purpose so the two are interchangeable in
//! the test scripts:
//!
//! ```text
//! nanite-bake-rs assets/models/bunny.gltf
//! nanite-bake-rs --rebuild assets/models/bunny.gltf
//! nanite-bake-rs --verify assets/models/bunny.fcc
//! ```
//!
//! The C++ builder stays in place; this binary is the first stage of the Rust port and is checked
//! against the C++ one by rendering both blobs in the same viewer.

mod bvh;
mod cluster;
mod emit;
mod gltf_load;
mod mesh;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};

/// Bake a glTF mesh into a Nanite `.fcc` blob.
#[derive(Debug, Parser)]
#[command(name = "nanite-bake-rs", version, about)]
struct Args {
	/// glTF files to bake, or `.fcc` files to verify with `--verify`.
	#[arg(value_name = "MODEL")]
	models: Vec<PathBuf>,

	/// Overwrite an existing blob.
	#[arg(long)]
	rebuild: bool,

	/// Validate an existing blob without building: header, versions, struct sizes, section table,
	/// and the strict source hash. Runs in a separate process from any build, which is the point.
	#[arg(long)]
	verify: bool,

	/// Write the blob somewhere other than `<model>.fcc` next to the mesh.
	#[arg(long, value_name = "PATH")]
	out: Option<PathBuf>,

	/// Skip the structural checks that the viewer would otherwise do for us.
	#[arg(long)]
	no_check: bool,

	/// Print the decoded payloads (BVH nodes, cluster/error records) of a blob. Debug aid for
	/// diffing a Rust bake against a C++ one - the two are not byte-identical, so the comparison
	/// has to happen on decoded values.
	#[arg(long)]
	dump: bool,

	/// `off`, `error`, `warn`, `info`, `debug` or `trace`.
	#[arg(long, default_value = "info", value_name = "LEVEL")]
	log: String,
}

fn main() -> ExitCode {
	let args = Args::parse();
	init_tracing(&args.log);

	if let Err(error) = run(&args) {
		// One line on stderr, exit code 1 - the C++ tool's contract with the test scripts.
		eprintln!("nanite-bake-rs: {error:#}");
		return ExitCode::FAILURE;
	}
	ExitCode::SUCCESS
}

fn init_tracing(level: &str) {
	let filter = match level.to_ascii_lowercase().as_str() {
		"off" => "off",
		"error" => "error",
		"warn" => "warn",
		"info" => "info",
		"debug" => "debug",
		"trace" => "trace",
		other => {
			eprintln!("nanite-bake-rs: unknown log level '{other}', using info");
			"info"
		}
	};
	let _ = tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
		)
		.with_target(false)
		.try_init();
}

fn run(args: &Args) -> Result<()> {
	if args.models.is_empty() {
		anyhow::bail!("no model given - usage: nanite-bake-rs [--rebuild] <model.gltf>...");
	}

	if args.verify {
		for model in &args.models {
			verify(model)?;
		}
		return Ok(());
	}

	if args.dump {
		for model in &args.models {
			dump(model)?;
		}
		return Ok(());
	}

	let mut failures = 0usize;
	for model in &args.models {
		match bake(model, args) {
			Ok(()) => {}
			Err(error) => {
				eprintln!("nanite-bake-rs: {}: {error:#}", model.display());
				failures += 1;
			}
		}
	}
	if failures > 0 {
		anyhow::bail!("{failures} of {} models failed", args.models.len());
	}
	Ok(())
}

/// `nanite_bake --verify`: read a blob back and report what is in it. A `.gltf` argument is
/// resolved to its sibling blob so both spellings work, exactly like the C++ tool.
fn verify(path: &Path) -> Result<()> {
	let blob = if nanite_blob::is_blob_path(path) {
		path.to_path_buf()
	} else {
		nanite_blob::blob_path_for(path)
	};
	if !blob.exists() {
		anyhow::bail!("no blob at {} - run without --verify to build it", blob.display());
	}

	let info = nanite_blob::inspect(&blob).with_context(|| format!("inspecting {}", blob.display()))?;
	println!("{}:", blob.display());
	println!(
		"  magic 0x{:08X}  version {}  builderVersion {}  size {}",
		info.header.magic, info.header.version, info.header.builder_version, info.file_size
	);
	println!(
		"  sourceHash {}  configHash {}  (expected configHash {})",
		info.header.source_hash,
		info.header.config_hash,
		nanite_blob::config_hash()
	);
	println!(
		"  struct sizes bvhNodeInfo {} clusterInfo {} errorInfo {} vertex {} blobBvhNode {}",
		info.header.size_bvh_node_info,
		info.header.size_cluster_info,
		info.header.size_error_info,
		info.header.size_vertex,
		info.header.size_blob_bvh_node
	);
	for (name, got, want) in [
		("sizeBVHNodeInfo", info.header.size_bvh_node_info, nanite_blob::SIZE_BVH_NODE_INFO),
		("sizeClusterInfo", info.header.size_cluster_info, nanite_blob::SIZE_CLUSTER_INFO),
		("sizeErrorInfo", info.header.size_error_info, nanite_blob::SIZE_ERROR_INFO),
		("sizeVertex", info.header.size_vertex, nanite_blob::SIZE_VERTEX),
		("sizeBlobBVHNode", info.header.size_blob_bvh_node, nanite_blob::SIZE_BLOB_BVH_NODE),
	] {
		if got != want {
			anyhow::bail!(
				"{} is {} but the C++ ABI requires {} - the blob would render garbage",
				name,
				got,
				want
			);
		}
	}
	for section in &info.sections {
		let name = section_name(section.key);
		println!(
			"  {name:<24} count {:>9}  offset {:>10}  size {:>10}",
			section.count, section.offset, section.size
		);
	}

	// The strict path: full validation, including re-hashing the source mesh.
	let asset = nanite_blob::read(&blob, nanite_blob::ReadOptions::default())
		.with_context(|| format!("validating {}", blob.display()))?;
	println!(
		"  OK: {} clusters, {} vertices, {} BVH nodes, {} LODs, source {}",
		asset.cluster_count(),
		asset.vertex_count(),
		asset.bvh_nodes.len(),
		asset.lod_nums,
		asset.source_path
	);
	Ok(())
}

/// Decode a blob and print the fields the shaders actually read, plus min/max statistics. Used
/// to compare a Rust bake against the C++ one when a render comes out wrong.
fn dump(path: &Path) -> Result<()> {
	let blob = if nanite_blob::is_blob_path(path) {
		path.to_path_buf()
	} else {
		nanite_blob::blob_path_for(path)
	};
	let options = nanite_blob::ReadOptions {
		strict_source_hash: false,
		..nanite_blob::ReadOptions::default()
	};
	let asset = nanite_blob::read(&blob, options).with_context(|| format!("reading {}", blob.display()))?;
	println!("{}:", blob.display());
	println!(
		"  {} clusters, {} indices ({} triangles), {} bvh nodes, {} roots {:?}, lodClusterNums {:?}",
		asset.cluster_infos.len(),
		asset.indices.len(),
		asset.indices.len() / 3,
		asset.bvh_nodes.len(),
		asset.bvh_roots.len(),
		asset.bvh_roots,
		asset.lod_cluster_nums
	);

	// BVH node statistics: the traversal gate is `errorWorld.y * screenBoundRadiusSq <= threshold`,
	// so a node whose errorWorld.y is not huge or whose parent sphere is not sane is never entered.
	let mut leaf = 0usize;
	let mut node = 0usize;
	let mut other = 0usize;
	let mut min_node_error = f32::MAX;
	let mut max_node_error = f32::MIN;
	let mut min_radius = f32::MAX;
	let mut max_radius = f32::MIN;
	let mut bad_range = 0usize;
	for n in &asset.bvh_nodes {
		match n.node_status {
			nanite_blob::node_status::LEAF => leaf += 1,
			nanite_blob::node_status::NODE => node += 1,
			_ => other += 1,
		}
		min_node_error = min_node_error.min(n.error_world[1]);
		max_node_error = max_node_error.max(n.error_world[1]);
		min_radius = min_radius.min(n.parent_bounding_sphere[3]);
		max_radius = max_radius.max(n.parent_bounding_sphere[3]);
		if n.node_status == nanite_blob::node_status::LEAF && n.start >= n.end {
			bad_range += 1;
		}
	}
	println!(
		"  bvh: {leaf} leaves, {node} nodes, {other} virtual/invalid, {bad_range} leaves with an empty range"
	);
	println!(
		"  bvh errorWorld.y range {min_node_error:.6} .. {max_node_error:.6}, parentBoundingSphere.w range {min_radius:.6} .. {max_radius:.6}"
	);
	for n in asset.bvh_roots.iter().take(8) {
		let n = &asset.bvh_nodes[*n as usize];
		println!(
			"  root {}: status {} depth {} start {} end {} errorWorld ({}, {}) sphere ({:.4} {:.4} {:.4} {:.4}) children {:?}",
			n.index,
			n.node_status,
			n.depth,
			n.start,
			n.end,
			n.error_world[0],
			n.error_world[1],
			n.parent_bounding_sphere[0],
			n.parent_bounding_sphere[1],
			n.parent_bounding_sphere[2],
			n.parent_bounding_sphere[3],
			n.children
		);
	}
	for n in asset.bvh_nodes.iter().take(6) {
		println!(
			"  node {}: status {} depth {} lod {} start {} end {} aabb ({:.4} {:.4} {:.4})..({:.4} {:.4} {:.4}) errorWorld ({}, {}) sphere ({:.4} {:.4} {:.4} {:.4}) children {:?}",
			n.index,
			n.node_status,
			n.depth,
			n.lod_level,
			n.start,
			n.end,
			n.p_min[0],
			n.p_min[1],
			n.p_min[2],
			n.p_max[0],
			n.p_max[1],
			n.p_max[2],
			n.error_world[0],
			n.error_world[1],
			n.parent_bounding_sphere[0],
			n.parent_bounding_sphere[1],
			n.parent_bounding_sphere[2],
			n.parent_bounding_sphere[3],
			n.children
		);
	}

	// Cluster statistics: `culling.comp:277` keeps a cluster iff
	// `errorWorld.x <= threshold && errorWorld.y > threshold`.
	let mut min_lod_error = f32::MAX;
	let mut max_lod_error = f32::MIN;
	let mut min_parent_error = f32::MAX;
	let mut max_parent_error = f32::MIN;
	let mut min_center_r = f32::MAX;
	let mut max_center_r = f32::MIN;
	let mut min_center_rp = f32::MAX;
	let mut max_center_rp = f32::MIN;
	let mut empty_aabb = 0usize;
	for (cluster, error) in asset.cluster_infos.iter().zip(asset.error_infos.iter()) {
		min_lod_error = min_lod_error.min(error.error_world[0]);
		max_lod_error = max_lod_error.max(error.error_world[0]);
		min_parent_error = min_parent_error.min(error.error_world[1]);
		max_parent_error = max_parent_error.max(error.error_world[1]);
		min_center_r = min_center_r.min(error.center_r[3]);
		max_center_r = max_center_r.max(error.center_r[3]);
		min_center_rp = min_center_rp.min(error.center_rp[3]);
		max_center_rp = max_center_rp.max(error.center_rp[3]);
		if cluster.p_min_world[0] > cluster.p_max_world[0] {
			empty_aabb += 1;
		}
	}
	println!("  clusters with an inverted AABB: {empty_aabb}");
	println!(
		"  cluster errorWorld.x range {min_lod_error:.6} .. {max_lod_error:.6}, .y range {min_parent_error:.6} .. {max_parent_error:.6}"
	);
	println!(
		"  cluster centerR.w range {min_center_r:.6} .. {max_center_r:.6}, centerRP.w range {min_center_rp:.6} .. {max_center_rp:.6}"
	);
	for i in 0..asset.cluster_infos.len().min(4) {
		println!(
			"  cluster {i}: tri {}..{} aabb ({:.4} {:.4} {:.4})..({:.4} {:.4} {:.4}) error ({}, {}) centerR ({:.4} {:.4} {:.4} {:.4}) centerRP ({:.4} {:.4} {:.4} {:.4})",
			asset.cluster_infos[i].triangle_indices_start,
			asset.cluster_infos[i].triangle_indices_end,
			asset.cluster_infos[i].p_min_world[0],
			asset.cluster_infos[i].p_min_world[1],
			asset.cluster_infos[i].p_min_world[2],
			asset.cluster_infos[i].p_max_world[0],
			asset.cluster_infos[i].p_max_world[1],
			asset.cluster_infos[i].p_max_world[2],
			asset.error_infos[i].error_world[0],
			asset.error_infos[i].error_world[1],
			asset.error_infos[i].center_r[0],
			asset.error_infos[i].center_r[1],
			asset.error_infos[i].center_r[2],
			asset.error_infos[i].center_r[3],
			asset.error_infos[i].center_rp[0],
			asset.error_infos[i].center_rp[1],
			asset.error_infos[i].center_rp[2],
			asset.error_infos[i].center_rp[3]
		);
	}
	Ok(())
}

fn section_name(key: u32) -> &'static str {
	match key {
		nanite_blob::section::VERTICES => "VERTICES",
		nanite_blob::section::INDICES => "INDICES",
		nanite_blob::section::CLUSTER_INFO => "CLUSTER_INFO",
		nanite_blob::section::ERROR_INFO => "ERROR_INFO",
		nanite_blob::section::BVH_NODES => "BVH_NODES",
		nanite_blob::section::SORTED_CLUSTER_INDICES => "SORTED_CLUSTER_INDICES",
		nanite_blob::section::LOD_CLUSTER_NUMS => "LOD_CLUSTER_NUMS",
		nanite_blob::section::BVH_ROOTS => "BVH_ROOTS",
		nanite_blob::section::SOURCE_PATH => "SOURCE_PATH",
		_ => "UNKNOWN",
	}
}

fn bake(path: &Path, args: &Args) -> Result<()> {
	if nanite_blob::is_blob_path(path) {
		anyhow::bail!(
			"{} is a blob; pass the source mesh to bake",
			path.display()
		);
	}
	let out = args
		.out
		.clone()
		.unwrap_or_else(|| nanite_blob::blob_path_for(path));
	if out.exists() && !args.rebuild {
		eprintln!(
			"nanite-bake-rs: {} already exists - pass --rebuild to overwrite",
			out.display()
		);
		anyhow::bail!("blob exists");
	}

	let bar = ProgressBar::new_spinner();
	bar.set_style(
		ProgressStyle::with_template("{spinner:.cyan} {msg} ({elapsed_precise})")
			.unwrap_or_else(|_| ProgressStyle::default_spinner()),
	);
	bar.enable_steady_tick(std::time::Duration::from_millis(120));

	bar.set_message(format!("loading {}", path.display()));
	let started = std::time::Instant::now();
	// The glTF is loaded through the same flag set the C++ builder uses: vertices pre-transformed
	// by the node matrix, Y flipped, vertex colours pre-multiplied. Baking with a different set
	// yields a mirrored mesh that still passes every hash check.
	let loaded = gltf_load::load_first_mesh(path)
		.with_context(|| format!("loading {}", path.display()))?;
	tracing::info!(
		"loaded node {:?}: {} vertices, {} indices",
		loaded.node_name,
		loaded.vertices.len(),
		loaded.indices.len()
	);
	// Parity note: `NaniteMesh::modelMatrix` is stored the same way and is equally never read by
	// the bake - `loadvkglTFModel` already applied it to every vertex (`PreTransformVertices`), so
	// applying it again would scale the model twice.
	tracing::debug!(
		"node matrix of {:?} is informational only (vertices are pre-transformed): {:?}",
		loaded.node_name, loaded.node_matrix
	);

	bar.set_message("building clusters");
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

	bar.set_message("building BVH");
	bvh::build_bvh(&mut lod);
	tracing::info!("BVH: {} nodes", lod.bvh.as_ref().unwrap().nodes.len());

	bar.set_message("writing blob");
	let asset = emit::build_asset(&loaded, &lod, path);
	if !args.no_check {
		if let Err(problem) = emit::check_asset(&asset) {
			anyhow::bail!("blob would not load: {problem}");
		}
	}
	// `sourceHash` over the *source mesh* file, the same bytes the C++ builder hashes. A missing
	// file hashes to 0, which tells `read()` to skip the strict check - deliberate, so a shipped
	// blob whose mesh is gone still loads.
	let source_hash = nanite_blob::fnv1a_file(path);
	nanite_blob::write(&out, &asset, source_hash).with_context(|| format!("writing {}", out.display()))?;

	bar.finish_and_clear();
	let lod0 = lod.clusters.len();
	println!(
		"{}: {} triangles -> {lod0} clusters, {} BVH nodes -> {} ({} bytes, {:.1} s)",
		path.display(),
		lod.mesh.face_count(),
		lod.bvh.as_ref().unwrap().nodes.len() + 1,
		out.display(),
		out.metadata().map(|m| m.len()).unwrap_or(0),
		started.elapsed().as_secs_f64()
	);
	Ok(())
}
