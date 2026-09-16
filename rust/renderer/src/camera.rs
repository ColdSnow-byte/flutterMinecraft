//! 第一人称相机：yaw / pitch + 透视投影。

use glam::{Mat4, Vec3};

const PITCH_LIMIT: f32 = 1.5533; // ≈ 89°

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// 眼睛所在的世界坐标。
    pub position: Vec3,
    /// 水平朝向，绕 +Y 轴，0 表示看向 -Z。
    pub yaw: f32,
    /// 俯仰角，向上为正。
    pub pitch: f32,
    pub fov_y: f32,
}

impl Camera {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            yaw: 0.0,
            // 初始俯视，让准星落在地面上（进入游戏就能看到选中方块的高亮边框）。
            pitch: -0.45,
            fov_y: 70f32.to_radians(),
        }
    }

    /// 鼠标移动：向右转 / 向下看。
    pub fn rotate(&mut self, delta_x: f32, delta_y: f32, sensitivity: f32) {
        self.yaw += delta_x * sensitivity;
        self.pitch = (self.pitch - delta_y * sensitivity).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        // 让 yaw 保持在 [-π, π]，避免长时间转动后浮点精度下降。
        if self.yaw > std::f32::consts::PI {
            self.yaw -= std::f32::consts::TAU;
        } else if self.yaw < -std::f32::consts::PI {
            self.yaw += std::f32::consts::TAU;
        }
    }

    /// 视线方向（单位向量）。
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    /// 水平前方（用于移动）。
    pub fn forward_horizontal(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos())
    }

    /// 水平右方。
    pub fn right(&self) -> Vec3 {
        self.forward_horizontal().cross(Vec3::Y)
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_to_rh(self.position, self.forward(), Vec3::Y)
    }

    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        // WebGPU 的裁剪空间深度范围是 [0, 1]，glam 的 *_rh 正好匹配。
        Mat4::perspective_rh(self.fov_y, aspect.max(0.01), 0.05, 512.0)
    }
}
