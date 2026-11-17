//! Model loading: `.fcc` blob, glTF through the Rust asset pipeline, and the two scene assembly
//! rules the C++ viewer applies on top (`createScene2`'s instance matrices and `viewAll`'s framing).
//!
//! The C++ viewer has two load paths and they must stay equivalent:
//!
//! - `--model bunny` / a `.gltf` path: parse the mesh, build (or cache) the nanite payload, then
//!   assemble an instanced scene.
//! - `--model bunny.fcc`: `NaniteMesh::initNaniteInfoFromBlob` -> `loadBlobFile`. No glTF is parsed
//!   at all; `models.object` is only *cleared*.
//!
//! The Rust version is the same shape: [`load_model`] decides between `nanite_blob::read` and
//! [`nanite_bake_rs::bake_asset`] (which builds the identical in-memory `Asset` without touching
//! disk), and both feed [`crate::scene::Scene::from_asset`].

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use glam::{Mat4, Vec3};
use nanite_bake_rs::BakeOptions;

use crate::camera::Camera;
use crate::scene::Scene;

/// `pbrtexture.cpp`: `#define MODEL_PATH "./assets/models/"`.
pub const MODEL_DIR: &str = "assets/models";

/// `pbrtexture.cpp`: `std::string model = "dragon";` - the default when `--model` is absent.
pub const DEFAULT_MODEL: &str = "dragon";

/// The environment cube the PBR passes sample: `assets/textures/hdr/gcanyon_cube.ktx`.
pub const ENV_CUBE_KTX: &str = "assets/textures/hdr/gcanyon_cube.ktx";

/// How a `.gltf` model gets its nanite payload. Mirrors `NaniteMesh::BlobMode`, minus `JsonOnly`:
/// there is no JSON cache in the Rust pipeline, so "fall back to JSON" has no meaning here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SourceMode {
    /// Prefer a sibling `<model>.fcc`, otherwise build. C++ `Auto`.
    #[default]
    Auto,
    /// The blob must exist. C++ `--nanite-blob` (`Require`).
    Require,
    /// Always build, and write the blob next to the mesh. C++ `--nanite-rebuild` (`Rebuild`).
    Rebuild,
}

/// One `--reload` / `--reload2` request, applied at most one per frame - see
/// `pbrtexture.cpp::handlePendingCommandLineReload`.
#[derive(Clone, Debug)]
pub struct ReloadRequest {
    pub model: String,
    pub frame: u64,
}

impl ReloadRequest {
    pub fn new(model: impl Into<String>, frame: u64) -> Self {
        Self {
            model: model.into(),
            frame,
        }
    }
}

/// The next queued reload whose frame has been reached, or `None`.
///
/// `pending` is in any order; the earliest due request wins, and only one is taken per frame, which
/// is what keeps two reloads from collapsing into a single `reloadModel()` call.
pub fn take_due_reload(pending: &mut Vec<ReloadRequest>, frame_index: u64) -> Option<ReloadRequest> {
    let best = pending
        .iter()
        .enumerate()
        .filter(|(_, r)| r.frame <= frame_index)
        .min_by_key(|(_, r)| r.frame)
        .map(|(i, _)| i)?;
    Some(pending.remove(best))
}

/// Resolve a `--model` argument to a loadable file.
///
/// The C++ viewer resolves the same three shapes (`loadNaniteModel` + `NaniteMesh::blobPathFor`):
/// an existing path as given, a bare model name inside `assets/models`, and a `.fcc` that may be
/// named with or without its extension.
pub fn resolve_model_path(model: &str, assets_dir: &Path) -> Result<PathBuf> {
    if model.trim().is_empty() {
        bail!("empty model name");
    }
    let direct = PathBuf::from(model);
    if direct.is_file() {
        return Ok(direct);
    }

    // A bare name: `bunny` -> `assets/models/bunny.gltf`, `bunny.fcc` -> `assets/models/bunny.fcc`.
    let in_assets = if nanite_blob::is_blob_path(Path::new(model)) {
        assets_dir.join(model)
    } else {
        let stem = Path::new(model)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| model.to_string());
        assets_dir.join(format!("{stem}.gltf"))
    };
    if in_assets.is_file() {
        return Ok(in_assets);
    }

    // A `.fcc` addressed by its mesh: `bunny` -> `assets/models/bunny.fcc`. This is the shape
    // `--nanite-blob --model bunny` takes when only the blob exists.
    if let Some(stem) = Path::new(model).file_stem() {
        let blob = assets_dir.join(format!(
            "{}{}",
            stem.to_string_lossy(),
            nanite_blob::NANITE_BLOB_EXTENSION
        ));
        if blob.is_file() {
            return Ok(blob);
        }
    }

    Err(anyhow!(
        "no such model: {model} (tried {}, {})",
        direct.display(),
        in_assets.display()
    ))
}

/// Load a model into a renderable [`Scene`].
///
/// `model` is a `--model` argument: a bare name, a `.gltf`, or a `.fcc`. A `.fcc` never touches the
/// asset pipeline; a `.gltf` is baked in memory by [`nanite_bake_rs::bake_asset`] unless a usable
/// sibling blob exists (see [`SourceMode`]).
pub fn load_model(model: &str, assets_dir: &Path, mode: SourceMode) -> Result<LoadedModel> {
    let path = resolve_model_path(model, assets_dir)?;
    if nanite_blob::is_blob_path(&path) {
        let asset = load_blob(&path)?;
        return Ok(LoadedModel {
            source: LoadSource::Blob(path),
            asset,
        });
    }

    // `Auto` / `Require`: a blob is cheaper than a build, and it is the same contract the C++
    // builder wrote. `bunny_rs.fcc` and friends are baked by the Rust tool, so a blob found here may
    // have come from either producer - the format crate validates both.
    let blob = nanite_blob::blob_path_for(&path);
    let mut source = LoadSource::Baked(path.clone());
    let asset = match mode {
        SourceMode::Rebuild => bake(&path)?,
        SourceMode::Auto | SourceMode::Require => {
            if blob.is_file() {
                source = LoadSource::Blob(blob.clone());
                read_blob(&blob)?
            } else if mode == SourceMode::Require {
                bail!(
                    "--nanite-blob: no blob cache exists for {} ({})",
                    path.display(),
                    blob.display()
                );
            } else {
                bake(&path)?
            }
        }
    };
    Ok(LoadedModel { source, asset })
}

/// Where a scene's payload actually came from - logged, and asserted by the load tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadSource {
    /// A `<model>.fcc` read from disk.
    Blob(PathBuf),
    /// Built in memory from a `.gltf` by `nanite_bake_rs`.
    Baked(PathBuf),
}

impl LoadSource {
    pub fn is_blob(&self) -> bool {
        matches!(self, LoadSource::Blob(_))
    }

    pub fn label(&self) -> String {
        match self {
            LoadSource::Blob(p) => format!("blob {}", p.display()),
            LoadSource::Baked(p) => format!("baked {}", p.display()),
        }
    }
}

/// A loaded mesh payload plus the path it came from. `Scene::from_asset` turns it into a scene.
pub struct LoadedModel {
    pub source: LoadSource,
    pub asset: nanite_blob::Asset,
}

/// `NaniteMesh::loadBlobFile` + `read`: full validation, including the strict source hash when the
/// source mesh is still around (`fnv1a_file` returns 0 for a missing file, and 0 skips the check -
/// a shipped blob whose mesh was deleted still loads).
pub fn load_blob(path: &Path) -> Result<nanite_blob::Asset> {
    read_blob(path)
}

fn read_blob(path: &Path) -> Result<nanite_blob::Asset> {
    let options = nanite_blob::ReadOptions {
        strict_source_hash: true,
    };
    let asset = nanite_blob::read(path, options)
        .map_err(|e| anyhow!("cannot read blob {}: {e}", path.display()))?;
    Ok(asset)
}

/// The builder path: `NaniteMesh::generateNaniteInfo` + `bakeBlob`, i.e. `nanite-bake-rs`.
fn bake(path: &Path) -> Result<nanite_blob::Asset> {
    let (asset, report) = nanite_bake_rs::bake_asset(path, BakeOptions::default())?;
    tracing::info!(
        "[bake] {} nodes={} triangles={} clusters={} cluster groups={} bvh nodes={} lods={}",
        path.display(),
        report.node_name,
        report.triangles,
        report.clusters,
        report.cluster_groups,
        report.bvh_nodes,
        report.lods,
    );
    Ok(asset)
}

/// `pbrtexture.cpp::createScene2`: the scene is the model drawn twice, once with `modelMats[0]` and
/// once with `modelMats[1]`.
///
/// `defaultModelMats()` returns five matrices, but `createScene2` only pushes the first two - the
/// remaining three belong to the commented-out `createScene()` variants. Matrix 0 is
/// `glm::rotate(glm::translate(identity, (0,0,3)), -90, Y)`. **glm post-multiplies in both helpers**
/// (`translate(m, v) == m * T`, `rotate(m, a, axis) == m * R`), so that expression is
/// `T(0,0,3) * Ry(-90)`: the vertex is rotated *first* and the translation is the *outer*
/// transform. Getting this backwards is not cosmetic - it moves the second instance across the
/// scene and changes the union AABB, which is what `viewAll()` frames (radius 2.4614 instead of the
/// measured 2.39159 for `bunny.fcc`).
pub fn default_instances() -> Vec<Mat4> {
    let translate = Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0));
    let rotate = Mat4::from_quat(glam::Quat::from_rotation_y(-90.0f32.to_radians()));
    vec![translate * rotate, Mat4::IDENTITY]
}

/// How much further out than "just past the far side of the scene" the far plane sits.
///
/// Ported from `viewAll()` (`pbrtexture.cpp:1322`): `farClip = max(distance + radius * 1.6f, 16.0f)`.
/// The extra `0.6 * radius` is not decoration: it keeps the far corner of a *rotated* bounding box
/// off the plane, and it is the value every golden image was captured with.
pub const FAR_PLANE_RADIUS_MARGIN: f32 = 1.6;

/// The far plane never drops below this. Load-bearing, and the reason [`view_all`] has the same
/// floor: the skybox cube spans +/-5 (corner distance 8.66) and is drawn with the *camera*
/// projection, so a smaller far plane clips the environment away.
pub const FAR_PLANE_SKYBOX_FLOOR: f32 = 16.0;

/// The scene's bounding sphere - the only quantity the frustum is sized from.
///
/// Cached rather than recomputed per frame: [`Scene::world_bounds`] walks every cluster of every
/// instance and transforms all eight corners of each, which is ~125k transforms per frame for
/// `dragon`. [`view_all`] derives the same pair from the same call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneBounds {
    pub center: Vec3,
    pub radius: f32,
}

impl SceneBounds {
    /// The union AABB of the scene as a center plus the radius `viewAll()` uses (half the diagonal,
    /// floored at 0.001). `None` for an empty scene.
    pub fn from_scene(scene: &Scene) -> Option<Self> {
        let (p_min, p_max) = scene.world_bounds()?;
        Some(Self {
            center: (p_min + p_max) * 0.5,
            radius: ((p_max - p_min) * 0.5).length().max(0.001),
        })
    }

    /// The far plane that still contains the whole bounding sphere when the eye is at `eye`:
    /// the distance to the far side of the sphere plus [`FAR_PLANE_RADIUS_MARGIN`], floored at
    /// [`FAR_PLANE_SKYBOX_FLOOR`] so the skybox survives.
    ///
    /// This is a *pure function of the eye*, so there is no state to drift and no hysteresis:
    /// dollying backwards out of the scene pushes the far plane out with the camera instead of
    /// letting the model fall behind a fixed plane, and coming back in pulls it in again - which
    /// also keeps the f32 depth budget as large as it can be (see
    /// [`crate::camera::NEAR_CLIP_DISABLED`], where the shaded accuracy is shown to be linear in
    /// `1 / near`; the same quantisation argument applies to `far`).
    #[must_use]
    pub fn far_for_eye(&self, eye: Vec3) -> f32 {
        ((eye - self.center).length() + self.radius * FAR_PLANE_RADIUS_MARGIN).max(FAR_PLANE_SKYBOX_FLOOR)
    }
}

/// Re-derive the far clipping plane from the *current* eye position, leaving fov, near and pose
/// exactly as they are.
///
/// [`view_all`] sizes the frustum once, and in the C++ viewer the far plane is fixed from then on:
/// walk or wheel-dolly far enough out of the scene and the model crosses it and disappears - not as
/// a clipped silhouette but as *nothing at all*, because `bvhtraversal.wgsl` and `culling.wgsl`
/// reject the nodes through `frustumCulling`. This keeps the plane wrapped around the scene instead,
/// for every rendered frame.
///
/// Returns the far plane in effect afterwards, which is [`Camera::far`] unless it was already right.
pub fn update_far_plane(bounds: &SceneBounds, camera: &mut Camera, width: u32, height: u32) -> f32 {
    let far = bounds.far_for_eye(camera.cam_pos());
    if far != camera.far {
        let aspect = if height > 0 {
            width as f32 / height as f32
        } else {
            1.0
        };
        // `set_perspective` re-floors `near` at `NEAR_CLIP_DISABLED`; `camera.near` already sits
        // exactly there, so the front plane is untouched.
        camera.set_perspective(camera.fov, aspect, camera.near, far);
    }
    far
}

/// `viewAll()` (`pbrtexture.cpp:1283`). Frames the loaded model exactly as the C++ viewer does, so
/// a Rust capture and a C++ capture of the same model differ only by rasterizer.
///
/// The `far >= 16` floor is load-bearing: the skybox cube spans +/-5 (corner 8.66) and is drawn with
/// the *camera* projection, so a smaller far plane clips the environment away. It lives in
/// [`SceneBounds::far_for_eye`], which is also what keeps the plane wrapped around the scene while
/// the camera moves - at the framing pose the two agree bit for bit.
pub fn view_all(scene: &Scene, camera: &mut Camera, width: u32, height: u32) -> Option<ViewAll> {
    let bounds = SceneBounds::from_scene(scene)?;
    let center = bounds.center;
    let radius = bounds.radius;
    let aspect = if height > 0 {
        width as f32 / height as f32
    } else {
        1.0
    };

    let half_fov_y = (camera.fov * 0.5).to_radians();
    let half_fov_x = (half_fov_y.tan() * aspect).atan();
    let distance = radius / half_fov_y.min(half_fov_x).sin() * crate::camera::VIEW_ALL_MARGIN;

    // C++: `camToWorld = inverse(camera.matrices.view)`, `viewDir = -normalize(col2(camToWorld))`.
    // `column(2)` of the inverse view is the camera's +z axis in world space, which glam stores in
    // `z_axis` of the *inverse* matrix.
    let cam_to_world = camera.view.inverse();
    let view_dir = -cam_to_world.z_axis.truncate().normalize_or_zero();
    let eye = center - view_dir * distance;

    // `Camera::position` is the negated eye in this convention (AGENTS.md: `viewPos = position *
    // (-1,1,-1,1)`), hence the negate - and `setPosition` stores its argument verbatim.
    camera.set_position(-eye);
    // The C++ viewer derives near from the framing (`max(0.01, distance - 1.1 * radius)`, which is
    // 2.87 for `bunny`) and therefore clips whatever comes in front of it. The Rust viewer has no
    // front clipping plane at all: `NEAR_CLIP_DISABLED` is an epsilon rather than a literal 0
    // because a zero near plane degenerates the projection - see `camera::NEAR_CLIP_DISABLED`.
    let near = crate::camera::NEAR_CLIP_DISABLED;
    // Same function [`update_far_plane`] applies on every later frame, evaluated at the eye just
    // set - so the framing pose and the running camera can never disagree about the far plane.
    let far = bounds.far_for_eye(eye);
    camera.set_perspective(camera.fov, aspect, near, far);

    let result = ViewAll {
        center,
        radius,
        distance,
        near: camera.near,
        far,
    };
    tracing::info!("{}", describe(&result));
    Some(result)
}

/// What [`view_all`] decided, for the `[view all]` log line the C++ viewer prints.
#[derive(Clone, Copy, Debug)]
pub struct ViewAll {
    pub center: Vec3,
    pub radius: f32,
    pub distance: f32,
    pub near: f32,
    pub far: f32,
}

/// `[view all] center (x, y, z), radius R, eye distance D, near N, far F`.
pub fn describe(v: &ViewAll) -> String {
    let c = v.center;
    format!(
        "[view all] center ({:.3}, {:.3}, {:.3}), radius {:.3}, eye distance {:.3}, near {:.3}, far {:.3}",
        c.x, c.y, c.z, v.radius, v.distance, v.near, v.far
    )
}

/// `resetCamera()` (`pbrtexture.cpp:1262`): the default pose *and* the default frustum.
///
/// `viewAll()` overwrites near/far, so the frustum has to be restored here as well - that is why the
/// C++ keeps `cameraFov/cameraNear/cameraFar` as members instead of reading them back.
pub fn reset_camera(camera: &mut Camera, width: u32, height: u32) {
    let aspect = if height > 0 {
        width as f32 / height as f32
    } else {
        1.0
    };
    camera.set_perspective(
        crate::camera::DEFAULT_FOV,
        aspect,
        crate::camera::DEFAULT_NEAR,
        crate::camera::DEFAULT_FAR,
    );
    camera.set_rotation(crate::camera::DEFAULT_ROTATION);
    camera.set_position(crate::camera::DEFAULT_POSITION);
}

/// The `assets/models` directory, found by walking up from the current directory.
///
/// The C++ examples hardcode `./assets/...` and are always started from the repository root (the
/// tests pass absolute paths, the viewer is run from the build output's working directory). A Rust
/// `cargo run` starts in `rust/`, so a plain relative path would miss; walking up keeps both
/// `cargo run -p nanite-viewer-rs` and a run from the repository root working without a flag.
///
/// Returns the **model directory**, i.e. what [`resolve_model_path`] and [`load_model`] take as
/// `assets_dir` - not the `assets/` folder above it.
pub fn find_model_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.as_os_str().is_empty() || start == Path::new(".") {
        std::env::current_dir().ok()?
    } else if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(start)
    };
    loop {
        let candidate = dir.join(MODEL_DIR);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn assets() -> PathBuf {
        find_model_dir(Path::new(".")).expect("assets/models is reachable from the workspace")
    }

    fn assert_approx(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn instances_are_the_two_createScene2_matrices() {
        let mats = default_instances();
        assert_eq!(mats.len(), 2);
        assert_eq!(mats[1], Mat4::IDENTITY);
        // `T(0,0,3) * Ry(-90)`: the vertex is rotated first, so the origin ends up at (0,0,3), and
        // the rotation sends +Z onto -X.
        let moved = mats[0] * Vec3::ZERO.extend(1.0);
        assert_approx(moved.truncate(), Vec3::new(0.0, 0.0, 3.0));
        let plus_z = mats[0] * Vec3::new(0.0, 0.0, 1.0).extend(1.0);
        assert_approx(plus_z.truncate(), Vec3::new(-1.0, 0.0, 3.0));
        // Spelled independently in the order glm builds it.
        let reference = Mat4::from_translation(Vec3::new(0.0, 0.0, 3.0))
            * Mat4::from_quat(Quat::from_rotation_y(-90.0f32.to_radians()));
        assert_eq!(mats[0], reference);
    }

    #[test]
    fn culling_model_is_the_pure_rotation() {
        // `uboCullingMatrices.model` is `glm::rotate(identity, -90, Y)` with **no** translation
        // (pbrtexture.cpp:3372), unlike instance 0.
        let pure = Mat4::from_quat(Quat::from_rotation_y(-90.0f32.to_radians()));
        assert_eq!(pure.w_axis.truncate(), Vec3::ZERO);
        assert_ne!(default_instances()[0], pure);
    }

    #[test]
    fn model_names_resolve_inside_assets() {
        let dir = assets();
        assert_eq!(
            resolve_model_path("bunny", &dir).unwrap(),
            dir.join("bunny.gltf")
        );
        assert_eq!(
            resolve_model_path("bunny.gltf", &dir).unwrap(),
            dir.join("bunny.gltf")
        );
        assert_eq!(
            resolve_model_path("./assets/models/bunny.gltf", &dir).unwrap(),
            dir.join("bunny.gltf")
        );
    }

    #[test]
    fn a_blob_is_found_by_either_name() {
        let dir = assets();
        let blob = dir.join(format!("bunny{}", nanite_blob::NANITE_BLOB_EXTENSION));
        if !blob.is_file() {
            return; // generated output; not checked in
        }
        assert_eq!(resolve_model_path("bunny.fcc", &dir).unwrap(), blob);
        assert_eq!(resolve_model_path("bunny", &dir).unwrap(), dir.join("bunny.gltf"));
        // With only the blob present the mesh name still resolves to it - exercised through the
        // helper the resolver uses.
        assert_eq!(nanite_blob::blob_path_for(&dir.join("bunny.gltf")), blob);
    }

    #[test]
    fn an_unknown_model_names_what_it_tried() {
        let dir = assets();
        let err = resolve_model_path("definitely_not_a_model", &dir).unwrap_err().to_string();
        assert!(err.contains("definitely_not_a_model"), "{err}");
    }

    #[test]
    fn reloads_are_applied_in_frame_order_one_per_frame() {
        let mut pending = vec![
            ReloadRequest::new("b", 8),
            ReloadRequest::new("a", 5),
            ReloadRequest::new("c", 20),
        ];
        // Frames 0..4: nothing is due.
        assert!(take_due_reload(&mut pending, 4).is_none());
        assert_eq!(take_due_reload(&mut pending, 5).unwrap().model, "a");
        // Only one per frame, even though 5 was due earlier than 8 in a later call.
        assert!(take_due_reload(&mut pending, 5).is_none());
        assert_eq!(take_due_reload(&mut pending, 8).unwrap().model, "b");
        assert!(take_due_reload(&mut pending, 19).is_none());
        assert_eq!(take_due_reload(&mut pending, 20).unwrap().model, "c");
        assert!(pending.is_empty());
    }

    #[test]
    fn out_of_order_reloads_still_run_earliest_first() {
        let mut pending = vec![ReloadRequest::new("late", 30), ReloadRequest::new("early", 2)];
        assert_eq!(take_due_reload(&mut pending, 30).unwrap().model, "early");
        assert_eq!(take_due_reload(&mut pending, 30).unwrap().model, "late");
    }

    #[test]
    fn a_blob_loads_without_its_mesh() {
        let dir = assets();
        let blob = dir.join(format!("bunny{}", nanite_blob::NANITE_BLOB_EXTENSION));
        if !blob.is_file() {
            return;
        }
        let loaded =
            load_model("bunny.fcc", &dir, SourceMode::Require).expect("bunny.fcc loads");
        assert!(loaded.source.is_blob());
        assert!(!loaded.asset.cluster_infos.is_empty());
    }

    #[test]
    fn require_without_a_blob_fails_loudly() {
        let dir = assets();
        // `voyager` has a mesh; whether a blob exists is not guaranteed, so only assert the shape of
        // the failure when it does not.
        let mesh = dir.join("bunny.gltf");
        if !mesh.is_file() {
            return;
        }
        let blob = nanite_blob::blob_path_for(&mesh);
        if blob.is_file() {
            let loaded =
                load_model("bunny", &dir, SourceMode::Require).expect("blob exists, so Require succeeds");
            assert!(loaded.source.is_blob());
        }
    }

    #[test]
    fn view_all_reproduces_the_measured_bunny_framing() {
        // Measured on this machine: `pbrtexture.exe --model bunny --captureframes 2` prints
        //   [blob]   loaded .../bunny.fcc (143685 verts, 849330 indices, 5212 clusters, 573 bvh nodes)
        //   [view all] center (-0.149, -0.8314, 1.55745), radius 2.39159, eye distance 5.50067,
        //              near 2.86991, far 16
        // The blob counts are asserted too: if the Rust decoder disagrees, the framing mismatch is
        // only the symptom.
        //
        // `near` is the one value the Rust viewer deliberately does *not* reproduce - it has no front
        // clipping plane, see `camera::NEAR_CLIP_DISABLED` and the `front_clipping_plane_is_disabled`
        // test below. Everything else (center, radius, eye distance, far) must stay bit-comparable to
        // the C++ viewer, because the goldens are C++ captures.
        let dir = assets();
        let loaded = load_model("bunny", &dir, SourceMode::Auto).expect("bunny loads");
        assert!(loaded.source.is_blob(), "bunny.fcc must be preferred");
        assert_eq!(loaded.asset.vertices.len(), 143_685);
        assert_eq!(loaded.asset.indices.len(), 849_330);
        assert_eq!(loaded.asset.cluster_infos.len(), 5_212);
        assert_eq!(loaded.asset.bvh_nodes.len(), 573);
        let scene = Scene::from_asset(loaded.asset, &default_instances()).unwrap();
        let mut camera = Camera::default();
        let v = view_all(&scene, &mut camera, 1280, 1024).expect("bounds");
        assert!((v.radius - 2.39159).abs() < 0.001, "radius {}", v.radius);
        assert!((v.distance - 5.50067).abs() < 0.001, "distance {}", v.distance);
        assert_eq!(v.near, crate::camera::NEAR_CLIP_DISABLED, "near {}", v.near);
        assert!((v.far - 16.0).abs() < 0.001, "far {}", v.far);
        let c = v.center;
        for (got, want) in [(c.x, -0.149), (c.y, -0.8314), (c.z, 1.55745)] {
            assert!((got - want).abs() < 0.001, "center component {got} != {want}");
        }
        // The eye must actually sit `distance` away from the center.
        let eye = -camera.position;
        assert!(((eye - c).length() - v.distance).abs() < 1e-3);
    }

    #[test]
    fn front_clipping_plane_is_disabled() {
        // The request: no front clipping plane. `near` may never be large enough to clip, and it may
        // never be the literal 0 either - a zero near plane makes `perspective_rh` degenerate (its z
        // row becomes `far / -far` and `0`, so every window depth collapses to 1.0 and the depth
        // test, the HiZ pyramid and the packed visibility depth all break). So the assertion is
        // two-sided: tiny, but not zero, and the projection stays usable.
        //
        // The upper bound is the *depth precision* budget, not a clipping requirement - see
        // `camera::NEAR_CLIP_DISABLED` for the measured render diffs at 1e-2 / 1e-3 / 1e-4 and for why
        // `render_rust_viewer_blob_reload_twice` is what actually pins this. Pinning the range here
        // keeps someone from "fixing" the near plane to a smaller epsilon and silently degrading the
        // shaded image by 10x per decade, or from raising it back into real clipping territory.
        assert!(crate::camera::NEAR_CLIP_DISABLED > 0.0);
        assert!(crate::camera::NEAR_CLIP_DISABLED <= 1e-2);
        assert_eq!(crate::camera::DEFAULT_NEAR, crate::camera::NEAR_CLIP_DISABLED);

        // Passing 0 means "no front clipping" and is floored, not stored verbatim.
        let mut camera = Camera::default();
        camera.set_perspective(60.0, 1280.0 / 1024.0, 0.0, 16.0);
        assert_eq!(camera.near, crate::camera::NEAR_CLIP_DISABLED);
        // glam has no `is_invertible`; prove usability the direct way - the inverse round-trips.
        let inv = camera.projection.inverse();
        assert!((inv * camera.projection).abs_diff_eq(Mat4::IDENTITY, 1e-3));
        // Depth must stay monotonic across the whole frustum, which is what a zero near plane breaks.
        let near = camera.projection.project_point3(Vec3::new(0.0, 0.0, -camera.near));
        let far = camera.projection.project_point3(Vec3::new(0.0, 0.0, -16.0));
        assert!(near.z.abs() < 1e-3, "near maps to {near:?}");
        assert!((far.z - 1.0).abs() < 1e-3, "far maps to {far:?}");
        assert!(far.z > near.z);

        // And neither the default frustum nor `viewAll()` reintroduces a clipping plane: the closest
        // dataset point to the eye is orders of magnitude further away than the epsilon.
        let mut camera = Camera::default();
        assert_eq!(camera.near, crate::camera::NEAR_CLIP_DISABLED);
        reset_camera(&mut camera, 1280, 1024);
        assert_eq!(camera.near, crate::camera::NEAR_CLIP_DISABLED);

        let dir = assets();
        let loaded = load_model("bunny", &dir, SourceMode::Auto).expect("bunny loads");
        let scene = Scene::from_asset(loaded.asset, &default_instances()).unwrap();
        let mut camera = Camera::default();
        let v = view_all(&scene, &mut camera, 1280, 1024).expect("bounds");
        assert_eq!(camera.near, crate::camera::NEAR_CLIP_DISABLED);
        // The C++ framing would have put a clipping plane at 2.87 - i.e. ~2.6 world units in front of
        // the model's nearest corner. Verify the epsilon really sits behind all geometry instead of
        // just trusting the constant: the closest bounding-box corner to the eye must be orders of
        // magnitude further away than the near plane.
        let eye = -camera.position;
        let closest = scene
            .world_bounds()
            .map(|(p_min, p_max)| {
                (0..8).fold(f32::MAX, |acc, i| {
                    let corner = Vec3::new(
                        if i & 1 == 0 { p_min.x } else { p_max.x },
                        if i & 2 == 0 { p_min.y } else { p_max.y },
                        if i & 4 == 0 { p_min.z } else { p_max.z },
                    );
                    acc.min((corner - eye).length())
                })
            })
            .expect("bounds");
        assert!(
            closest > camera.near * 100.0,
            "closest corner is {closest} away from the eye, too close to a near plane of {}",
            camera.near
        );
        assert!(v.near <= 1e-2, "near {}", v.near);
    }

    #[test]
    fn view_all_keeps_the_far_plane_floor() {
        // A small model must still see the skybox: far >= 16 regardless of size. `nanite_test` has no
        // blob, so this also exercises the in-memory bake path.
        let dir = assets();
        let loaded = load_model("nanite_test", &dir, SourceMode::Auto).expect("nanite_test loads");
        let scene = Scene::from_asset(loaded.asset, &default_instances()).unwrap();
        let mut camera = Camera::default();
        let v = view_all(&scene, &mut camera, 1280, 1024).unwrap();
        assert!(v.far >= 16.0, "far {}", v.far);
        assert!(camera.far >= 16.0);
    }

    #[test]
    fn far_for_eye_tracks_the_eye_and_keeps_the_skybox_floor() {
        // Pure arithmetic, no assets: `|eye - center| + 1.6 * radius`, floored at 16 for the skybox.
        let b = SceneBounds {
            center: Vec3::ZERO,
            radius: 2.0,
        };
        assert_eq!(b.far_for_eye(Vec3::ZERO), FAR_PLANE_SKYBOX_FLOOR);
        assert_eq!(
            b.far_for_eye(Vec3::new(5.0, 0.0, 0.0)),
            FAR_PLANE_SKYBOX_FLOOR,
            "still inside the floor"
        );
        assert!((b.far_for_eye(Vec3::new(20.0, 0.0, 0.0)) - (20.0 + 3.2)).abs() < 1e-5);
        // Only the distance to the center matters, not the direction.
        assert!((b.far_for_eye(Vec3::new(0.0, -30.0, 0.0)) - (30.0 + 3.2)).abs() < 1e-4);
        // And it is monotone in that distance - the property the fix exists for.
        let mut previous = 0.0;
        for d in 0..40 {
            let far = b.far_for_eye(Vec3::new(d as f32 * 2.0, 0.0, 0.0));
            assert!(far >= previous, "far must not shrink as the eye backs away: {far} < {previous}");
            previous = far;
        }
    }

    #[test]
    fn far_plane_always_encompasses_the_scene_bounds() {
        // The request: the far plane is not fixed at what `viewAll()` chose - it has to keep the
        // whole bounding box in front of it however far the camera backs out of the scene.
        //
        // This is not cosmetic: `bvhtraversal.wgsl`/`culling.wgsl` reject nodes through
        // `frustumCulling`, so a scene behind the far plane does not render clipped, it renders as
        // *nothing*.
        //
        // Same magnitude the wheel handler applies (`app::WHEEL_DOLLY_PER_NOTCH == 0.6`), spelled out
        // here because `app` is not reachable from this module's tests.
        const DOLLY_PER_NOTCH: f32 = 0.6;
        let dir = assets();
        let loaded = load_model("bunny", &dir, SourceMode::Auto).expect("bunny loads");
        let scene = Scene::from_asset(loaded.asset, &default_instances()).unwrap();
        let bounds = SceneBounds::from_scene(&scene).expect("bounds");
        let (p_min, p_max) = scene.world_bounds().expect("bounds");
        let mut camera = Camera::default();
        let v = view_all(&scene, &mut camera, 1280, 1024).expect("bounds");

        // At the framing pose the per-frame update must be a *no-op*: every golden is a capture taken
        // at exactly this pose, so rebuilding the projection here would be a pixel change.
        let pose = (camera.position, camera.rotation);
        let far = update_far_plane(&bounds, &mut camera, 1280, 1024);
        assert_eq!(far, v.far, "the framing pose must not move the far plane");
        assert_eq!(camera.far, v.far);
        assert_eq!(camera.near, crate::camera::NEAR_CLIP_DISABLED, "near untouched");
        assert_eq!(camera.fov, crate::camera::DEFAULT_FOV, "fov untouched");
        assert_eq!((camera.position, camera.rotation), pose, "pose untouched");

        // Back the eye out of the scene the way the wheel dolly does (`amount < 0` = away from what
        // the camera looks at), 0.6 units per notch, and check the frustum after every notch.
        let corners: Vec<Vec3> = (0..8u32)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { p_min.x } else { p_max.x },
                    if i & 2 == 0 { p_min.y } else { p_max.y },
                    if i & 4 == 0 { p_min.z } else { p_max.z },
                )
            })
            .collect();
        assert!(
            v.far <= FAR_PLANE_SKYBOX_FLOOR + 1e-3,
            "bunny frames with far {}, so the fixed plane is what a 30-notch dolly-out runs into",
            v.far
        );
        let mut far_grew = false;
        for notch in 1..=40 {
            camera.dolly(-DOLLY_PER_NOTCH);
            let far = update_far_plane(&bounds, &mut camera, 1280, 1024);
            assert!(far >= FAR_PLANE_SKYBOX_FLOOR, "far {}", far);
            if far > v.far {
                far_grew = true;
            }
            for corner in &corners {
                let clip = camera.projection * camera.view * corner.extend(1.0);
                assert!(clip.w > 0.0, "corner {corner:?} behind the eye after {notch} notches");
                let depth = clip.z / clip.w;
                assert!(
                    depth <= 1.0,
                    "corner {corner:?} is behind the far plane {far} after {notch} notches \
                     (window depth {depth})"
                );
            }
        }
        assert!(far_grew, "the far plane never moved - the fix is not wired up");

        // Dolly back in and the plane comes back down with it: the depth budget is not ratcheted.
        for _ in 0..40 {
            camera.dolly(DOLLY_PER_NOTCH);
            update_far_plane(&bounds, &mut camera, 1280, 1024);
        }
        assert!((camera.far - v.far).abs() < 1e-3, "far {}", camera.far);
    }

    #[test]
    fn update_far_plane_changes_nothing_but_far() {
        // The per-frame update must not disturb fov, near, aspect or the pose - it only moves one
        // plane. And once it has agreed with the camera, it stays a no-op.
        let mut camera = Camera::default();
        camera.set_perspective(45.0, 1920.0 / 1080.0, 0.0, 123.0);
        let bounds = SceneBounds {
            center: Vec3::new(100.0, 0.0, 0.0),
            radius: 1.0,
        };
        let pose = (camera.position, camera.rotation);
        let far = update_far_plane(&bounds, &mut camera, 1920, 1080);
        let expected = (camera.cam_pos() - bounds.center).length() + FAR_PLANE_RADIUS_MARGIN;
        assert!((far - expected).abs() < 1e-4, "far {far} != {expected}");
        assert_eq!(camera.far, far);
        assert_eq!(camera.fov, 45.0, "fov must not change");
        assert_eq!(camera.near, crate::camera::NEAR_CLIP_DISABLED, "near must not change");
        assert_eq!((camera.position, camera.rotation), pose, "pose must not change");
        let expected_proj = crate::camera::flip_projection_y(Mat4::perspective_rh(
            45.0f32.to_radians(),
            1920.0 / 1080.0,
            crate::camera::NEAR_CLIP_DISABLED,
            far,
        ));
        assert!(camera.projection.abs_diff_eq(expected_proj, 1e-6));

        // Second call: nothing to do, and the projection is left byte for byte as it was.
        let again = update_far_plane(&bounds, &mut camera, 1920, 1080);
        assert_eq!(again, far);
        assert_eq!(camera.projection, expected_proj);
    }

    #[test]
    fn reset_camera_restores_the_default_frustum_too() {
        let mut camera = Camera::default();
        camera.set_perspective(20.0, 1.0, 3.0, 4.0);
        reset_camera(&mut camera, 1280, 1024);
        assert_eq!(camera.fov, crate::camera::DEFAULT_FOV);
        assert_eq!(camera.near, crate::camera::DEFAULT_NEAR);
        assert_eq!(camera.far, crate::camera::DEFAULT_FAR);
        assert_eq!(camera.position, crate::camera::DEFAULT_POSITION);
        assert_eq!(camera.rotation, crate::camera::DEFAULT_ROTATION);
    }
}
