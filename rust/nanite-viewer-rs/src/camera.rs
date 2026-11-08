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
/// `cameraNear`.
pub const DEFAULT_NEAR: f32 = 0.1;
/// `cameraFar`.
pub const DEFAULT_FAR: f32 = 256.0;
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
