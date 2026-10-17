//! 2D orthographic camera with Unity semantics.

use glam::{Mat4, Vec2, Vec3};

/// A 2D orthographic camera, like a Unity camera in orthographic mode.
///
/// World space is y-up. `ortho_size` is half the visible height in world
/// units (Unity's `orthographicSize`); the visible width follows from the
/// target's aspect ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera2D {
    /// World position of the view centre.
    pub position: Vec2,
    /// Half the visible height in world units.
    pub ortho_size: f32,
    /// Rotation in radians (counter-clockwise rotation of the camera).
    pub rotation: f32,
    /// Extra offset in world units added to `position`; screen shake goes
    /// here so it never shows the edge of an intermediate texture.
    pub shake: Vec2,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            ortho_size: 5.0,
            rotation: 0.0,
            shake: Vec2::ZERO,
        }
    }
}

impl Camera2D {
    pub fn new(position: Vec2, ortho_size: f32) -> Self {
        Self {
            position,
            ortho_size,
            ..Default::default()
        }
    }

    /// Half-extents of the visible world rectangle for a given aspect
    /// (width / height).
    pub fn half_extents(&self, aspect: f32) -> Vec2 {
        Vec2::new(self.ortho_size * aspect, self.ortho_size)
    }

    /// World-to-clip matrix for a target with the given aspect.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let h = self.half_extents(aspect);
        let proj = Mat4::orthographic_rh(-h.x, h.x, -h.y, h.y, -1.0, 1.0);
        let eye = self.position + self.shake;
        let view = Mat4::from_rotation_z(-self.rotation) * Mat4::from_translation(-eye.extend(0.0));
        proj * view
    }

    /// Target pixels per world unit for a target of the given pixel height.
    pub fn pixels_per_unit(&self, target_height: u32) -> f32 {
        target_height as f32 / (2.0 * self.ortho_size)
    }

    /// Converts a pixel position (origin top-left, y down) to world space.
    pub fn screen_to_world(&self, pixel: Vec2, target: (u32, u32)) -> Vec2 {
        let (w, h) = (target.0 as f32, target.1 as f32);
        let ndc = Vec2::new(pixel.x / w * 2.0 - 1.0, 1.0 - pixel.y / h * 2.0);
        let inv = self.view_proj(w / h).inverse();
        inv.project_point3(Vec3::new(ndc.x, ndc.y, 0.0)).truncate()
    }

    /// Converts a world position to pixels (origin top-left, y down).
    pub fn world_to_screen(&self, world: Vec2, target: (u32, u32)) -> Vec2 {
        let (w, h) = (target.0 as f32, target.1 as f32);
        let ndc = self.view_proj(w / h).project_point3(world.extend(0.0));
        Vec2::new((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h)
    }
}

/// Clip matrix for screen-space drawing: pixel coordinates with the origin
/// at the top-left and y pointing down.
pub fn screen_proj(width: u32, height: u32) -> Mat4 {
    Mat4::orthographic_rh(0.0, width as f32, height as f32, 0.0, -1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cam = Camera2D {
            position: Vec2::new(3.0, -2.0),
            ortho_size: 10.0,
            rotation: 0.3,
            shake: Vec2::ZERO,
        };
        let p = Vec2::new(4.5, 1.25);
        let s = cam.world_to_screen(p, (1920, 1080));
        let back = cam.screen_to_world(s, (1920, 1080));
        assert!((back - p).length() < 1e-3);
    }

    #[test]
    fn centre_maps_to_middle() {
        let cam = Camera2D::new(Vec2::new(1.0, 1.0), 5.0);
        let s = cam.world_to_screen(Vec2::new(1.0, 1.0), (800, 600));
        assert!((s - Vec2::new(400.0, 300.0)).length() < 1e-3);
        // Top edge of the view is ortho_size above the centre.
        let top = cam.world_to_screen(Vec2::new(1.0, 6.0), (800, 600));
        assert!(top.y.abs() < 1e-3);
        assert_eq!(cam.pixels_per_unit(600), 60.0);
    }
}
