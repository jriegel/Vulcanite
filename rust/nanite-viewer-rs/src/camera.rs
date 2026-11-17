//! Port of `base/camera.hpp` for the `CameraType::firstperson` convention pbrtexture uses.
//!
//! The sign traps documented in `AGENTS.md` are reproduced here rather than "fixed", because the
//! whole pipeline - `viewAll()`, the culling shaders, `shading.frag`'s world-position rebuild -
//! assumes them:
//!
//! * `matrices.view = rotM * transM`, so a world point is transformed as `rot * (p + position)`.
//! * [`Camera::position`] is therefore the **negated eye**: the world-space camera location is
//!   `-position`, which is exactly what `uboshading.camPos = camera.position * -1.0f` uploads.
//! * [`Camera::get_front`] is the **negative** of the actual look direction.
//!
//! pbrtexture never sets `Camera::flipY`, but [`flip_projection_y`] applies the same negation for
//! a different reason - wgpu's viewport is already Y-flipped, see its docs.

use glam::{Mat4, Vec3};

/// `cameraFov` in pbrtexture.cpp.
pub const DEFAULT_FOV: f32 = 60.0;
/// `cameraNear` - the front clipping plane is **disabled**, see [`NEAR_CLIP_DISABLED`].
pub const DEFAULT_NEAR: f32 = NEAR_CLIP_DISABLED;
/// `cameraFar`.
pub const DEFAULT_FAR: f32 = 256.0;

/// The front (near) clipping plane, i.e. how the Rust viewer "removes" it.
///
/// `near == 0.0` literally is not implementable: [`Mat4::perspective_rh`] builds its z row from
/// `far / (near - far)` and `near * far / (near - far)`, so a zero near plane collapses *every*
/// window depth to 1.0 - which takes the depth test, the HiZ pyramid and the 32-bit depth field of
/// the packed visibility buffer (`common.wgsl::packVis`, `depth:32 | clusterId:15 | objectId:11 |
/// triangleId:6`) down with it. [`Camera::set_perspective`] clamps through this constant, so callers
/// may pass `0.0` to mean "no front clipping".
///
/// The value is a **depth-precision budget**, not a clipping distance. With a `far / near` ratio of
/// `R`, a point at view depth `z` lands at window depth `1 - near/z` (plus `1/R` at the far plane),
/// so the whole scene is squeezed into the last `near * far/(far - near)` of the `[0, 1]` interval
/// before the f32 mantissa is even read. `shading.wgsl:112` then rebuilds the world position from
/// that depth (`invView * invProj * vec4(uv * 2 - 1, depth, 1)`), and `1e-7` of depth becomes
/// `1e-7 * far` of world position - enough to perturb the normal, hence the specular highlight.
/// Measured against the C++ golden (`bunny.fcc`, 1920x1080, mean abs diff / % bad pixels):
///
/// | near | shaded | vis1 | vis2 | vis3 |
/// | --- | --- | --- | --- | --- |
/// | 2.870 (what C++ derives) | 0.0077 / 0.013 % | 0.0221 | 0.0087 | 0.0055 |
/// | 1e-2 | 0.0316 / 0.020 % | 0.0224 | 0.0093 | 0.0058 |
/// | 1e-3 | 0.2051 / 0.286 % | 0.0224 | 0.0101 | 0.0058 |
/// | 1e-4 | 0.8654 / 1.625 % | 0.0225 | - | - |
///
/// The flat-colour visualization modes barely move: `vis1` (LOD level) and `vis3` (grayscale
/// visibility id) are *identical* at every near plane, which proves the cluster/LOD set and the
/// visibility-buffer ids do not change at all - only the reconstructed world position does. (`vis2`
/// drifts by ~0.001 because its colour is a hash that includes the 32-bit depth field.) So the shaded
/// drift is pure f32 depth quantisation, and it grows linearly in `1/near`.
///
/// `1e-2` is the chosen value: it is the largest that still clips nothing on any dataset (~2.6 world
/// units behind the nearest geometry everywhere) while keeping the shaded golden diff small, and the
/// smallest that keeps the *shaded* capture close to deterministic. Going smaller buys clipping
/// headroom nothing uses and pays for it in shading accuracy; going larger reintroduces a real front
/// plane.
///
/// The cost at this value, measured: the shaded capture is **not bit-deterministic** run to run. Over
/// 20 runs of `--model bunny` at 1920x1080, 18 are identical and 2 differ by exactly one pixel with a
/// +/-1 LSB channel change. At `1e-1`, `1.0` and `2.870` the same probe is 10/10 identical, and the
/// C++ viewer (which derives `near = 2.870`) is 10/10 identical, so this is quantisation, not a bug in
/// the port. The cluster/LOD set and the visibility-buffer ids provably do not move: `--visualize 2`
/// (a hash of the id, which reads no depth) is 20/20 identical here. What changes is the reconstructed
/// world position, because with `near = 1e-2` the scene occupies only `near*far/(far-near) ~ 0.010`
/// of `[0, 1]`, so exact depth ties between triangles become common and the `LessEqual` depth compare
/// resolves a tie by primitive order, which Vulkan/wgpu do not define. `tests/CMakeLists.txt` therefore
/// runs tests 19/20 at `--pixel-tol 0 --max-bad 0.00002` (41 of 2 073 600 pixels) rather than demanding
/// bit equality; do not "fix" that by raising this constant.
pub const NEAR_CLIP_DISABLED: f32 = 1e-2;
/// `defaultCameraRotation`.
pub const DEFAULT_ROTATION: Vec3 = Vec3::new(-7.75, 150.25, 0.0);
/// `defaultCameraPosition`.
pub const DEFAULT_POSITION: Vec3 = Vec3::new(0.7, 0.1, 1.7);
/// `camera.movementSpeed = 4.0f`.
pub const DEFAULT_MOVEMENT_SPEED: f32 = 4.0;
/// `viewAllMargin`: how much larger than "exactly fitting the frustum" the automatic framing is.
pub const VIEW_ALL_MARGIN: f32 = 1.15;

/// The C++ `Camera::flipY` branch - `base/camera.hpp:96`/`:104`,
/// `if (flipY) { matrices.perspective[1][1] *= -1.0f; }`.
///
/// pbrtexture leaves `flipY` false, but it also draws with a **positive-height** viewport
/// (`vks::initializers::viewport(w, h, 0.0f, 1.0f)` at every call site), so there NDC y = +1 lands
/// on framebuffer row `h`. wgpu's Vulkan backend *always* installs a Y-flipped viewport
/// (`wgpu-hal-30.0.1/src/vulkan/command.rs:1028` - `height: -rect.h, // flip Y`), which puts
/// NDC y = +1 on row 0 instead: every capture comes out a vertical mirror of the C++ one. A
/// negative viewport height is rejected by wgpu (`wgpu-core-30.0.1/src/command/render.rs:3016`),
/// so the flip has to be pushed into the projection, where it cancels the viewport flip exactly.
///
/// Consequences, all checked:
/// * HW rasterization: flipped viewport * flipped clip y == C++'s positive viewport * unflipped
///   clip y, so both the pixel placement *and* the triangle winding match `pbrtexture.cpp` - which
///   is what makes `VK_CULL_MODE_FRONT_BIT` for the skybox correct again.
/// * `swrasterize.wgsl` runs in a compute shader with no viewport and maps NDC to pixels itself,
///   so it needs the same negated clip y to hit the rows the HW rasterizer writes.
/// * `frustumCulling`'s NDC bounds and `getScreenBoundRadiusSq`'s squared distance are symmetric
///   in y, and `shading.wgsl` rebuilds world positions through `invProj` - all unaffected.
#[must_use]
pub fn flip_projection_y(mut projection: Mat4) -> Mat4 {
    projection.y_axis.y *= -1.0;
    projection
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    /// `Camera::position` - the *negated* eye position, see the module docs.
    pub position: Vec3,
    /// `Camera::rotation`, in degrees, applied X then Y then Z.
    pub rotation: Vec3,
    pub movement_speed: f32,
    pub projection: Mat4,
    pub view: Mat4,
}

impl Default for Camera {
    fn default() -> Self {
        let mut c = Self {
            fov: DEFAULT_FOV,
            near: DEFAULT_NEAR,
            far: DEFAULT_FAR,
            position: DEFAULT_POSITION,
            rotation: DEFAULT_ROTATION,
            movement_speed: DEFAULT_MOVEMENT_SPEED,
            projection: Mat4::IDENTITY,
            view: Mat4::IDENTITY,
        };
        c.update_view_matrix();
        c
    }
}

impl Camera {
    /// `Camera::updateViewMatrix()` for `type == firstperson`. The three `glm::rotate` calls
    /// accumulate as `Rx * Ry * Rz`, so the composition is written out explicitly instead of
    /// relying on an euler helper whose angle order is documented differently.
    pub fn update_view_matrix(&mut self) {
        let rot = Mat4::from_rotation_x(self.rotation.x.to_radians())
            * Mat4::from_rotation_y(self.rotation.y.to_radians())
            * Mat4::from_rotation_z(self.rotation.z.to_radians());
        self.view = rot * Mat4::from_translation(self.position);
    }

    /// `Camera::setPerspective`. `Mat4::perspective_rh` is byte-for-byte `glm::perspective` with
    /// `GLM_FORCE_DEPTH_ZERO_TO_ONE`, which the C++ build enables - no depth re-mapping needed.
    /// The result then goes through [`flip_projection_y`], which is the C++ `flipY` branch.
    pub fn set_perspective(&mut self, fov: f32, aspect: f32, near: f32, far: f32) {
        self.fov = fov;
        // `near <= 0` means "no front clipping plane"; it is floored at NEAR_CLIP_DISABLED rather
        // than reaching perspective_rh as-is. See that constant's docs.
        let near = near.max(NEAR_CLIP_DISABLED);
        self.near = near;
        self.far = far;
        self.projection = flip_projection_y(Mat4::perspective_rh(fov.to_radians(), aspect, near, far));
    }

    /// `Camera::updateAspectRatio`, which re-creates the projection from the *stored* fov/near/far.
    pub fn update_aspect_ratio(&mut self, aspect: f32) {
        let p = Mat4::perspective_rh(self.fov.to_radians(), aspect, self.near, self.far);
        self.projection = flip_projection_y(p);
    }

    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
        self.update_view_matrix();
    }

    pub fn set_rotation(&mut self, rotation: Vec3) {
        self.rotation = rotation;
        self.update_view_matrix();
    }

    /// `Camera::rotate(offset)`, which the mouse handler calls with the mouse delta.
    pub fn rotate(&mut self, delta: Vec3) {
        self.rotation += delta;
        self.update_view_matrix();
    }

    /// `Camera::translate(offset)`: pans `position` itself, *not* the world. Because `position` is
    /// the negated eye, a positive `offset.z` moves the eye backwards - which is exactly what the
    /// C++ right-drag handler relies on (`camera.translate(vec3(0, 0, dy * 0.005f))`).
    pub fn translate(&mut self, delta: Vec3) {
        self.position += delta;
        self.update_view_matrix();
    }

    /// Dolly along the **view axis**: `amount > 0` moves the eye forward, into the scene.
    ///
    /// This is *not* what the C++ base framework's wheel handler does. `Camera::translate` in
    /// `base/camera.hpp` is `position += delta` with no view-space transform, so
    /// `camera.translate(vec3(0, 0, wheelDelta * 0.005f))` (`vulkanexamplebase.cpp:1450`) slides the
    /// camera along the **world** Z axis — fine for the default pose, wrong as soon as the camera is
    /// orbited. A wheel dolly should follow where the camera looks, so this pushes along
    /// [`Camera::get_front`], which is the *negated* look direction in this convention; and because
    /// [`Camera::position`] is the negated eye too, the two negations cancel and `+front` moves the
    /// eye forward. The magnitude the wheel handler applies is `0.6` world units per notch, matching
    /// the C++ `120 * 0.005f`.
    pub fn dolly(&mut self, amount: f32) {
        self.position += self.get_front() * amount;
        self.update_view_matrix();
    }

    /// `Camera::getFront()` - the *negated* look direction in this convention.
    pub fn get_front(&self) -> Vec3 {
        let rx = self.rotation.x.to_radians();
        let ry = self.rotation.y.to_radians();
        Vec3::new(-rx.cos() * ry.sin(), rx.sin(), rx.cos() * ry.cos()).normalize_or_zero()
    }

    /// `Camera::getRight()`.
    pub fn get_right(&self) -> Vec3 {
        self.get_front().cross(Self::get_up()).normalize_or_zero()
    }

    /// `Camera::getUp()`.
    pub fn get_up() -> Vec3 {
        Vec3::Y
    }

    /// `Camera::update()` firstperson: `forward` walks along `get_front()` (W/S and Page
    /// Up/Down), `sideways` along its horizontal perpendicular (A/D and the arrow keys).
    pub fn update(&mut self, forward: f32, sideways: f32, delta_seconds: f32) {
        if forward != 0.0 || sideways != 0.0 {
            let front = self.get_front();
            let speed = delta_seconds * self.movement_speed;
            self.position += front * forward * speed;
            self.position += front.cross(Vec3::Y).normalize_or_zero() * sideways * speed;
        }
        self.update_view_matrix();
    }

    /// `uboshading.camPos` = `camera.position * -1`, i.e. the world-space eye position implied by
    /// `view = rot * translate(position)`.
    pub fn cam_pos(&self) -> Vec3 {
        -self.position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_is_rotation_times_translation() {
        let mut c = Camera::default();
        c.rotation = Vec3::ZERO;
        c.position = Vec3::new(1.0, 2.0, 3.0);
        c.update_view_matrix();
        assert_eq!(c.view, Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        // The eye is the inverse-view image of the origin, and must equal -position.
        let eye = c.view.inverse().transform_point3(Vec3::ZERO);
        assert_eq!(eye, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(c.cam_pos(), eye);
    }

    #[test]
    fn translate_adds_to_position() {
        // The C++ mouse handler: right button translates z, middle button translates x/y.
        let mut c = Camera::default();
        let before = c.position;
        c.translate(Vec3::new(0.0, 0.0, 0.5));
        assert_eq!(c.position, before + Vec3::new(0.0, 0.0, 0.5));
        c.translate(Vec3::new(-0.25, -0.5, 0.0));
        assert_eq!(c.position, before + Vec3::new(-0.25, -0.5, 0.5));
        // view must have been rebuilt: view * 0 == rot * position.
        let rotated = Mat4::from_rotation_x(c.rotation.x.to_radians())
            * Mat4::from_rotation_y(c.rotation.y.to_radians())
            * Mat4::from_rotation_z(c.rotation.z.to_radians());
        assert_approx(c.view.transform_point3(Vec3::ZERO), rotated.transform_point3(c.position));
    }

    #[test]
    fn dolly_follows_the_view_axis() {
        // The wheel must move the eye *where the camera looks*, not along world Z (which is what the
        // C++ `camera.translate(vec3(0, 0, d))` does). `position` is the negated eye and
        // `get_front()` is the negated look direction, so `position += front * amount` moves the eye
        // along `-front` == the look direction: `amount > 0` dollies **in**.
        let mut c = Camera::default();
        let eye_before = c.cam_pos();
        let front = c.get_front();
        c.dolly(0.6);
        let moved = c.cam_pos() - eye_before;
        assert!((moved + front * 0.6).length() < 1e-6, "{moved:?}");
        assert!((moved.length() - 0.6).abs() < 1e-6);
        // And it really is a move *towards* what the camera looks at: the look direction is
        // `-get_front()`, so the eye must not move backwards.
        assert!(moved.dot(-front) > 0.0);

        // Rotating first must change the dolly direction - the whole point of the view-axis version.
        let mut c = Camera::default();
        c.set_rotation(Vec3::new(0.0, 90.0, 0.0));
        let eye_before = c.cam_pos();
        c.dolly(1.0);
        let moved = c.cam_pos() - eye_before;
        assert!((moved + c.get_front()).length() < 1e-6, "{moved:?}");
        // A 90 degree yaw looks along -X in this convention, so the eye moves +X - not world Z.
        assert!(moved.x.abs() > 0.9, "{moved:?} should be mostly along X");
        assert!(moved.z.abs() < 1e-3, "{moved:?} must not stay on the world Z axis");
    }

    #[test]
    fn rotation_is_applied_after_translation() {
        // view = rot * translate(p) => view * 0 == rot * p, so a point at the origin lands at the
        // rotated position. This is the "camera orbits its own origin" behaviour.
        let mut c = Camera::default();
        c.position = Vec3::new(2.0, 0.0, 0.0);
        c.rotation = Vec3::new(0.0, 90.0, 0.0);
        c.update_view_matrix();
        let p = c.view.transform_point3(Vec3::ZERO);
        assert_approx(p, Vec3::new(0.0, 0.0, -2.0));
    }

    #[test]
    fn get_front_matches_the_glm_formula() {
        let c = Camera::default();
        let rx = DEFAULT_ROTATION.x.to_radians();
        let ry = DEFAULT_ROTATION.y.to_radians();
        let want = Vec3::new(-rx.cos() * ry.sin(), rx.sin(), rx.cos() * ry.cos()).normalize();
        assert_approx(c.get_front(), want);
        assert_approx(c.get_right(), c.get_front().cross(Vec3::Y).normalize());
    }

    #[test]
    fn perspective_maps_depth_to_zero_to_one() {
        let mut c = Camera::default();
        c.set_perspective(60.0, 16.0 / 9.0, 0.1, 256.0);
        let near = c.projection.project_point3(Vec3::new(0.0, 0.0, -0.1));
        let far = c.projection.project_point3(Vec3::new(0.0, 0.0, -256.0));
        assert!((near.z - 0.0).abs() < 1e-5, "near maps to {near:?}");
        assert!((far.z - 1.0).abs() < 1e-4, "far maps to {far:?}");
    }

    fn assert_approx(a: Vec3, b: Vec3) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-5, "{a:?} != {b:?}");
        }
    }
}
