//! glTF loading, mirroring the exact transform chain the C++ builder consumes.
//!
//! The C++ side loads a mesh through `vkglTF::Model::loadFromFile` with
//! `NaniteMesh::kBuilderLoadingFlags` = `PreTransformVertices | PreMultiplyVertexColors | FlipY`
//! (`base/VulkanglTFModel.cpp`), and `NaniteMesh` then copies `vertexBuffer`/`indexBuffer` into an
//! OpenMesh mesh (`mesh/NaniteMesh.cpp`, `vkglTFPrimitiveToOpenMesh`). The order of operations per
//! vertex there is fixed and observable in the baked blob, so it is reproduced literally:
//!
//! 1. `pos = nodeMatrix * pos`, `normal = normalize(mat3(nodeMatrix) * normal)`
//!    (`PreTransformVertices`, `base/VulkanglTFModel.cpp:1275-1278`),
//! 2. `pos.y = -pos.y`, `normal.y = -normal.y` (`FlipY`, `:1281-1282`),
//! 3. `color = material.baseColorFactor * color` (`PreMultiplyVertexColors`, `:1284-1286`).
//!
//! Step 3 is irrelevant for the blob: `Mesh::initUniqueVertexBuffer` never copies `color`, so the
//! baked `color` is the zero-initialised default. It is still applied here so a future change to
//! that function cannot silently diverge.
//!
//! # What is *not* reproduced
//!
//! Skinning, morph targets and multiple primitives per mesh. `NaniteMesh::loadvkglTFModel` takes
//! the first node with a mesh and comments "TODO: Only support naniting one mesh within a model",
//! so the C++ builder is equally limited; the loader reports what it skipped instead of hiding it.

use std::path::Path;

use anyhow::{Context, Result, bail};
use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
use gltf::scene::Transform;
use tracing::debug;

use crate::mesh::TriangleMesh;

/// One vertex as it exists after the loading transforms - the payload
/// `vkglTFPrimitiveToOpenMesh` hands to OpenMesh.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadedVertex {
	pub pos: Vec3,
	pub normal: Vec3,
	pub uv: Vec2,
	/// Kept for parity with `vkglTF::Vertex::color`; never reaches the blob.
	pub color: Vec4,
}

/// A mesh extracted from a glTF file, in the coordinate system the builder expects.
#[derive(Clone, Debug)]
pub struct LoadedMesh {
	/// The node the mesh was found on.
	pub node_name: String,
	/// `NaniteMesh::modelMatrix` - the node's world matrix. The baked payloads are in the
	/// *already transformed* space, so this is informational only.
	pub node_matrix: Mat4,
	pub vertices: Vec<LoadedVertex>,
	/// Triangle indices into `vertices`, in file order.
	pub indices: Vec<u32>,
}

/// Load the first mesh of a glTF file, applying the builder's loading flags.
///
/// Accepts `.gltf` (JSON, with external or `data:` URI buffers) and `.glb`. Images are never
/// decoded, matching `FileLoadingFlags::DontLoadImages` in `tools/nanite_bake`.
pub fn load_first_mesh(path: &Path) -> Result<LoadedMesh> {
	let gltf = gltf::Gltf::open(path)
		.with_context(|| format!("cannot parse {}", path.display()))?;

	// `Gltf::open` resolves nothing but the JSON; buffers are fetched separately so images can be
	// skipped. `blob` carries the GLB payload for binary files.
	let base = path.parent();
	let buffer_data = gltf::import_buffers(&gltf.document, base, gltf.blob.clone())
		.with_context(|| format!("cannot load buffers of {}", path.display()))?;

	let get_buffer_data = |buffer: gltf::Buffer| Some(&buffer_data[buffer.index()].0[..]);

	let nodes = collect_nodes(&gltf.document);
	let Some(node) = nodes.iter().find(|n| n.mesh.is_some()) else {
		bail!(
			"{}: no node with a mesh ({} nodes scanned)",
			path.display(),
			nodes.len()
		);
	};

	let mesh_index = node.mesh.expect("checked above");
	let mesh = gltf
		.document
		.meshes()
		.nth(mesh_index)
		.with_context(|| format!("{}: mesh index {mesh_index} missing", path.display()))?;

	let material_factor = mesh
		.primitives()
		.next()
		.map(|p| base_color_factor(&p))
		.unwrap_or(Vec4::ONE);

	let mut out = LoadedMesh {
		node_name: node.name.clone(),
		node_matrix: node.world_matrix,
		vertices: Vec::new(),
		indices: Vec::new(),
	};

	for primitive in mesh.primitives() {
		if primitive.indices().is_none() {
			debug!(
				"{}: primitive without an index buffer skipped",
				path.display()
			);
			continue;
		}
		if primitive.mode() != gltf::mesh::Mode::Triangles {
			bail!(
				"{}: primitive draw mode {:?} is not supported - the nanite pipeline only \
				 rasterizes triangles",
				path.display(),
				primitive.mode()
			);
		}
		read_primitive(
			&primitive,
			&get_buffer_data,
			node.world_matrix,
			material_factor,
			&mut out,
		)
		.with_context(|| format!("{}: while reading a primitive", path.display()))?;
	}

	if out.vertices.is_empty() || out.indices.is_empty() {
		bail!(
			"{}: mesh on node '{}' has no triangle data",
			path.display(),
			out.node_name
		);
	}

	Ok(out)
}

/// A node of the scene graph after the hierarchy has been flattened.
struct FlatNode {
	name: String,
	world_matrix: Mat4,
	/// Index into `Document::meshes()`.
	mesh: Option<usize>,
}

/// Walk the default scene and flatten the hierarchy.
///
/// The order matters: `vkglTF::Model::loadNode` pushes into `linearNodes` *after* recursing into
/// the children (post-order), and `NaniteMesh::loadvkglTFModel` takes the first entry that has a
/// mesh. Collecting in the same order keeps "which mesh gets nanite-ified" identical.
fn collect_nodes(document: &gltf::Document) -> Vec<FlatNode> {
	let mut out = Vec::new();
	let scene = document
		.default_scene()
		.or_else(|| document.scenes().next());
	let Some(scene) = scene else {
		return out;
	};
	for node in scene.nodes() {
		walk_node(node, Mat4::IDENTITY, &mut out);
	}
	out
}

fn walk_node(node: gltf::Node, parent_world: Mat4, out: &mut Vec<FlatNode>) {
	let local = local_matrix(node.transform());
	let world = parent_world * local;

	for child in node.children() {
		walk_node(child, world, out);
	}

	out.push(FlatNode {
		name: node.name().unwrap_or("").to_string(),
		world_matrix: world,
		mesh: node.mesh().map(|m| m.index()),
	});
}

/// `vkglTF::Node::localMatrix()` = `translate * rotation * scale * matrix`.
///
/// glTF 2.0 makes `matrix` and TRS mutually exclusive, but vkglTF multiplies both, so this does
/// too - a file that carries both then bakes the same way in both implementations.
fn local_matrix(transform: Transform) -> Mat4 {
	let (t, r, s) = match transform {
		Transform::Matrix { .. } => ([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
		Transform::Decomposed {
			translation,
			rotation,
			scale,
		} => (translation, rotation, scale),
	};
	let matrix = match transform {
		Transform::Matrix { matrix } => Mat4::from_cols_array_2d(&matrix),
		Transform::Decomposed { .. } => Mat4::IDENTITY,
	};
	Mat4::from_translation(Vec3::from_array(t))
		* Mat4::from_quat(Quat::from_xyzw(r[0], r[1], r[2], r[3]))
		* Mat4::from_scale(Vec3::from_array(s))
		* matrix
}

fn base_color_factor(primitive: &gltf::Primitive) -> Vec4 {
	// `Primitive::material()` is infallible in `gltf` 1.4.1 - a primitive without a material gets a
	// default one, whose `baseColorFactor` is the spec default `[1,1,1,1]`. That matches vkglTF,
	// which multiplies by `material.baseColorFactor` and leaves unmaterialised primitives alone.
	Vec4::from_array(primitive.material().pbr_metallic_roughness().base_color_factor())
}

/// Read one primitive into `out`, rebasing its indices by the current vertex offset exactly like
/// `vkglTF::Model::loadNode` (`indexBuffer.push_back(buf[index] + vertexStart)`).
fn read_primitive<'s, F>(
	primitive: &gltf::Primitive,
	get_buffer_data: &F,
	world: Mat4,
	material_factor: Vec4,
	out: &mut LoadedMesh,
) -> Result<()>
where
	F: Fn(gltf::Buffer) -> Option<&'s [u8]>,
{
	let reader = primitive.reader(|b| get_buffer_data(b));

	let positions: Vec<[f32; 3]> = reader
		.read_positions()
		.context("POSITION is required")?
		.collect();
	let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|i| i.collect());
	let tex_coords: Option<Vec<[f32; 2]>> = reader
		.read_tex_coords(0)
		.map(|tc| match tc {
			gltf::mesh::util::ReadTexCoords::F32(i) => i.collect(),
			gltf::mesh::util::ReadTexCoords::U16(i) => {
				i.map(|c| [c[0] as f32 / 65535.0, c[1] as f32 / 65535.0]).collect()
			}
			gltf::mesh::util::ReadTexCoords::U8(i) => {
				i.map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0]).collect()
			}
		});
	let colors: Option<Vec<[f32; 4]>> = reader.read_colors(0).map(|c| match c {
		gltf::mesh::util::ReadColors::RgbaF32(i) => i.collect(),
		gltf::mesh::util::ReadColors::RgbF32(i) => {
			i.map(|c| [c[0], c[1], c[2], 1.0]).collect()
		}
		gltf::mesh::util::ReadColors::RgbaU8(i) => {
			i.map(|c| [
				c[0] as f32 / 255.0,
				c[1] as f32 / 255.0,
				c[2] as f32 / 255.0,
				c[3] as f32 / 255.0,
			])
			.collect()
		}
		gltf::mesh::util::ReadColors::RgbU8(i) => {
			i.map(|c| [
				c[0] as f32 / 255.0,
				c[1] as f32 / 255.0,
				c[2] as f32 / 255.0,
				1.0,
			])
			.collect()
		}
		gltf::mesh::util::ReadColors::RgbaU16(i) => {
			i.map(|c| [
				c[0] as f32 / 65535.0,
				c[1] as f32 / 65535.0,
				c[2] as f32 / 65535.0,
				c[3] as f32 / 65535.0,
			])
			.collect()
		}
		gltf::mesh::util::ReadColors::RgbU16(i) => {
			i.map(|c| [
				c[0] as f32 / 65535.0,
				c[1] as f32 / 65535.0,
				c[2] as f32 / 65535.0,
				1.0,
			])
			.collect()
		}
	});

	// vkglTF normalises whatever it finds, including the all-zero default when NORMAL is absent.
	// A zero normal is kept as zero rather than fabricated - the shaders only use it for shading.
	let normal_matrix = Mat3::from_mat4(world);
	let vertex_start = out.vertices.len() as u32;
	for i in 0..positions.len() {
		let mut v = LoadedVertex::default();
		v.pos = Vec3::from_array(positions[i]);
		v.normal = normals
			.as_ref()
			.map(|n| Vec3::from_array(n[i]))
			.unwrap_or(Vec3::ZERO);

		// 1. PreTransformVertices.
		v.pos = world.transform_point3(v.pos);
		v.normal = normal_matrix * v.normal;
		if v.normal.length_squared() > 0.0 {
			v.normal = v.normal.normalize();
		}
		// 2. FlipY.
		v.pos.y = -v.pos.y;
		v.normal.y = -v.normal.y;
		// 3. PreMultiplyVertexColors.
		v.color = colors
			.as_ref()
			.map(|c| Vec4::from_array(c[i]))
			.unwrap_or(Vec4::ONE)
			* material_factor;

		v.uv = tex_coords
			.as_ref()
			.map(|t| Vec2::from_array(t[i]))
			.unwrap_or(Vec2::ZERO);

		out.vertices.push(v);
	}

	let indices = match reader.read_indices() {
		Some(gltf::mesh::util::ReadIndices::U8(i)) => {
			i.map(u32::from).collect::<Vec<u32>>()
		}
		Some(gltf::mesh::util::ReadIndices::U16(i)) => {
			i.map(u32::from).collect::<Vec<u32>>()
		}
		Some(gltf::mesh::util::ReadIndices::U32(i)) => i.collect::<Vec<u32>>(),
		None => bail!("primitive has no index buffer"),
	};
	for index in indices {
		out.indices.push(index + vertex_start);
	}

	Ok(())
}

/// Turn a [`LoadedMesh`] into the builder's [`TriangleMesh`].
pub fn to_triangle_mesh(mesh: &LoadedMesh) -> TriangleMesh {
	let mut tri = TriangleMesh::new(
		mesh.vertices.iter().map(|v| v.pos).collect(),
		&mesh.indices,
	);
	tri.name = mesh.node_name.clone();
	tri
}
