//! 体素世界：地形生成、面剔除式网格构建、射线拾取（DDA）与 AABB 碰撞。

use glam::{IVec3, Vec3};

use crate::renderer::Vertex;

// 方块类型。
pub const AIR: u8 = 0;
pub const GRASS: u8 = 1;
pub const DIRT: u8 = 2;
pub const STONE: u8 = 3;
pub const SAND: u8 = 4;
pub const WOOD: u8 = 5;
pub const LEAVES: u8 = 6;

// 世界尺寸（方块数）。
pub const SIZE_X: i32 = 32;
pub const SIZE_Y: i32 = 16;
pub const SIZE_Z: i32 = 32;

/// 玩家碰撞盒：宽 0.6，高 1.8，与 Minecraft 一致。
pub const PLAYER_HALF_WIDTH: f32 = 0.3;
pub const PLAYER_HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.62;

/// 6 个面方向：+Y 顶面，-Y 底面，其余为侧面。
const FACE_DIRS: [[i32; 3]; 6] = [
    [0, 1, 0],
    [0, -1, 0],
    [1, 0, 0],
    [-1, 0, 0],
    [0, 0, 1],
    [0, 0, -1],
];

#[derive(Clone)]
pub struct World {
    blocks: Vec<u8>,
    /// 每次修改方块后自增，用来判断是否需要重建网格。
    pub revision: u64,
    /// 世界里的非空气方块数量，方便 UI 展示。
    pub solid_count: u32,
    /// 出生点（水平坐标），选在地形最平坦的地方。
    pub spawn: (i32, i32),
}

#[derive(Debug, Clone)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct RayHit {
    /// 命中的方块坐标。
    pub block: IVec3,
    /// 进入该方块的那一面（用于计算放置位置）。
    pub face: [i32; 3],
}

impl World {
    /// 生成一个带起伏地形、几棵树的世界。
    pub fn generate() -> Self {
        let mut world = Self {
            blocks: vec![AIR; (SIZE_X * SIZE_Y * SIZE_Z) as usize],
            revision: 0,
            solid_count: 0,
            spawn: (SIZE_X / 2, SIZE_Z / 2),
        };

        for x in 0..SIZE_X {
            for z in 0..SIZE_Z {
                let top = terrain_height(x, z);
                for y in 0..=top {
                    let id = if y == top {
                        if top <= 3 {
                            SAND
                        } else {
                            GRASS
                        }
                    } else if y >= top - 2 {
                        DIRT
                    } else {
                        STONE
                    };
                    world.set_raw(x, y, z, id);
                }
            }
        }

        // 先定出生点（此时还没种树，避免把出生点选在树冠上），再种树。
        world.spawn = world.flat_spawn();
        for (tx, tz) in tree_positions() {
            let ground = terrain_height(tx, tz);
            world.plant_tree(tx, ground + 1, tz);
        }

        world.recount();
        world
    }

    /// 找一个相对平坦的位置当出生点：3x3 高差越小越好，同时尽量靠近世界中心。
    fn flat_spawn(&self) -> (i32, i32) {
        let center = (SIZE_X / 2, SIZE_Z / 2);
        let mut best = center;
        let mut best_score = i32::MAX;

        for x in 2..SIZE_X - 2 {
            for z in 2..SIZE_Z - 2 {
                let height = self.surface_height(x, z);
                let mut difference = 0;
                for dx in -1i32..=1 {
                    for dz in -1i32..=1 {
                        difference =
                            difference.max((self.surface_height(x + dx, z + dz) - height).abs());
                    }
                }
                // 高差权重远大于离中心的距离，保证优先选平地。
                let score = difference * 6
                    + (x - center.0).abs()
                    + (z - center.1).abs();
                if score < best_score {
                    best_score = score;
                    best = (x, z);
                }
            }
        }

        best
    }

    #[inline]
    fn index(x: i32, y: i32, z: i32) -> usize {
        ((y * SIZE_Z + z) * SIZE_X + x) as usize
    }

    #[inline]
    fn in_bounds(x: i32, y: i32, z: i32) -> bool {
        x >= 0 && x < SIZE_X && y >= 0 && y < SIZE_Y && z >= 0 && z < SIZE_Z
    }

    /// 读取方块。世界外围视作实心墙、地面以下视作实心，这样玩家不会掉出去。
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        if y < 0 {
            return STONE;
        }
        if y >= SIZE_Y {
            return AIR;
        }
        if x < 0 || x >= SIZE_X || z < 0 || z >= SIZE_Z {
            return STONE;
        }
        self.blocks[Self::index(x, y, z)]
    }

    /// 射线拾取用的读取：世界外围的"隐形墙"不参与命中。
    #[inline]
    fn get_for_ray(&self, x: i32, y: i32, z: i32) -> u8 {
        if !Self::in_bounds(x, y, z) {
            return AIR;
        }
        self.blocks[Self::index(x, y, z)]
    }

    #[inline]
    pub fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        self.get(x, y, z) != AIR
    }

    fn set_raw(&mut self, x: i32, y: i32, z: i32, id: u8) {
        if Self::in_bounds(x, y, z) {
            self.blocks[Self::index(x, y, z)] = id;
        }
    }

    /// 修改方块（越界会忽略）。返回是否真的发生了变化。
    pub fn set(&mut self, x: i32, y: i32, z: i32, id: u8) -> bool {
        if !Self::in_bounds(x, y, z) {
            return false;
        }
        let index = Self::index(x, y, z);
        if self.blocks[index] == id {
            return false;
        }
        self.blocks[index] = id;
        if id == AIR {
            self.solid_count = self.solid_count.saturating_sub(1);
        } else {
            self.solid_count += 1;
        }
        self.revision += 1;
        true
    }

    fn recount(&mut self) {
        self.solid_count = self.blocks.iter().filter(|&&b| b != AIR).count() as u32;
    }

    /// 某一列最高的实心方块高度（世界外返回地面高度）。
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        for y in (0..SIZE_Y).rev() {
            if self.get(x, y, z) != AIR {
                return y;
            }
        }
        0
    }

    /// 在指定位置长一棵 4 格高的树。
    fn plant_tree(&mut self, x: i32, base_y: i32, z: i32) {
        let trunk_height = 4;
        for i in 0..trunk_height {
            self.set_raw(x, base_y + i, z, WOOD);
        }
        let top = base_y + trunk_height;
        for dy in -2i32..=1 {
            for dx in -2i32..=2 {
                for dz in -2i32..=2 {
                    // 用一个简单规则把树冠削成球形。
                    let r = dx * dx + dy * dy * 2 + dz * dz;
                    if r > 5 || (dx.abs() == 2 && dz.abs() == 2) {
                        continue;
                    }
                    let (lx, ly, lz) = (x + dx, top - 1 + dy, z + dz);
                    if Self::in_bounds(lx, ly, lz) && self.get(lx, ly, lz) == AIR {
                        self.set_raw(lx, ly, lz, LEAVES);
                    }
                }
            }
        }
    }

    /// 只为"暴露在空气中"的面生成几何体（面剔除）。
    pub fn build_mesh(&self) -> Mesh {
        let mut mesh = Mesh {
            vertices: Vec::new(),
            indices: Vec::new(),
        };

        for x in 0..SIZE_X {
            for y in 0..SIZE_Y {
                for z in 0..SIZE_Z {
                    let id = self.get(x, y, z);
                    if id == AIR {
                        continue;
                    }
                    let tint = block_tint(x, y, z);
                    for dir in FACE_DIRS {
                        let neighbor = self.get(x + dir[0], y + dir[1], z + dir[2]);
                        if neighbor != AIR {
                            continue;
                        }
                        push_face(&mut mesh, x, y, z, dir, block_color(id, dir), tint);
                    }
                }
            }
        }

        mesh
    }

    /// 从 `origin` 沿 `direction` 做体素遍历（Amanatides & Woo），返回第一个命中方块。
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<RayHit> {
        let dir = direction.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }

        let mut voxel = IVec3::new(
            origin.x.floor() as i32,
            origin.y.floor() as i32,
            origin.z.floor() as i32,
        );
        let step = IVec3::new(
            dir.x.signum() as i32,
            dir.y.signum() as i32,
            dir.z.signum() as i32,
        );

        // 每跨一整个体素需要的 t。
        let t_delta = Vec3::new(
            axis_delta(dir.x),
            axis_delta(dir.y),
            axis_delta(dir.z),
        );
        let mut t_max = Vec3::new(
            axis_max(origin.x, dir.x, step.x),
            axis_max(origin.y, dir.y, step.y),
            axis_max(origin.z, dir.z, step.z),
        );

        let mut last_face = [0, 0, 0];
        let mut travelled = 0.0f32;

        while travelled <= max_distance {
            if self.get_for_ray(voxel.x, voxel.y, voxel.z) != AIR {
                return Some(RayHit {
                    block: voxel,
                    face: last_face,
                });
            }

            // 前进到最近的体素边界。
            if t_max.x <= t_max.y && t_max.x <= t_max.z {
                voxel.x += step.x;
                travelled = t_max.x;
                t_max.x += t_delta.x;
                last_face = [-step.x, 0, 0];
            } else if t_max.y <= t_max.z {
                voxel.y += step.y;
                travelled = t_max.y;
                t_max.y += t_delta.y;
                last_face = [0, -step.y, 0];
            } else {
                voxel.z += step.z;
                travelled = t_max.z;
                t_max.z += t_delta.z;
                last_face = [0, 0, -step.z];
            }

            // 明确越界后就不用再走了。
            if voxel.x < -1
                || voxel.x > SIZE_X + 1
                || voxel.z < -1
                || voxel.z > SIZE_Z + 1
                || voxel.y < -1
                || voxel.y > SIZE_Y + 1
            {
                return None;
            }
        }

        None
    }

    /// 玩家碰撞盒（以脚部中心为基准的 AABB）是否与世界中的实心方块相交。
    pub fn collides(&self, feet: Vec3) -> bool {
        let min = Vec3::new(
            feet.x - PLAYER_HALF_WIDTH,
            feet.y,
            feet.z - PLAYER_HALF_WIDTH,
        );
        let max = Vec3::new(
            feet.x + PLAYER_HALF_WIDTH,
            feet.y + PLAYER_HEIGHT,
            feet.z + PLAYER_HALF_WIDTH,
        );

        // 收缩一点点，避免站在方块上时因为浮点误差被判成碰撞。
        const EPS: f32 = 1e-4;
        let x0 = (min.x + EPS).floor() as i32;
        let x1 = (max.x - EPS).floor() as i32;
        let y0 = (min.y + EPS).floor() as i32;
        let y1 = (max.y - EPS).floor() as i32;
        let z0 = (min.z + EPS).floor() as i32;
        let z1 = (max.z - EPS).floor() as i32;

        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    if self.is_solid(x, y, z) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn axis_delta(direction: f32) -> f32 {
    if direction == 0.0 {
        f32::INFINITY
    } else {
        (1.0 / direction).abs()
    }
}

fn axis_max(origin: f32, direction: f32, step: i32) -> f32 {
    if direction == 0.0 {
        return f32::INFINITY;
    }
    let boundary = if step > 0 {
        origin.floor() + 1.0
    } else {
        origin.floor()
    };
    (boundary - origin) / direction
}

/// 地形高度：用几个正弦叠加当作廉价噪声，起伏控制在 2~8 格，方便走动和搭建。
fn terrain_height(x: i32, z: i32) -> i32 {
    let (fx, fz) = (x as f32, z as f32);
    let noise = (fx * 0.26).sin() + (fz * 0.29).cos() + ((fx + fz) * 0.15).sin();
    (5.0 + noise * 1.05).round() as i32
}

fn tree_positions() -> Vec<(i32, i32)> {
    vec![(6, 7), (24, 9), (9, 23), (26, 25), (16, 5), (5, 16)]
}

/// 每个方块轻微的颜色抖动，让同种方块不至于完全一样。
fn block_tint(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ z.wrapping_mul(1_274_126_177))
        as u32;
    h ^= h >> 13;
    h = h.wrapping_mul(1_274_126_177);
    let v = ((h >> 9) & 0xFF) as f32 / 255.0;
    0.93 + v * 0.14
}

/// 方块在某个朝向上的颜色。
fn block_color(id: u8, dir: [i32; 3]) -> [f32; 3] {
    let top = dir[1] > 0;
    let bottom = dir[1] < 0;
    match id {
        GRASS => {
            if top {
                [0.36, 0.68, 0.28]
            } else if bottom {
                [0.48, 0.36, 0.25]
            } else {
                // 侧面：上面一小条草色，其余是泥土（这里用混合近似）。
                [0.42, 0.55, 0.27]
            }
        }
        DIRT => [0.48, 0.36, 0.25],
        STONE => [0.55, 0.55, 0.58],
        SAND => [0.83, 0.77, 0.52],
        WOOD => {
            if top || bottom {
                [0.62, 0.48, 0.29]
            } else {
                [0.42, 0.32, 0.20]
            }
        }
        LEAVES => [0.24, 0.52, 0.21],
        _ => [0.7, 0.7, 0.7],
    }
}

/// 生成一个面的 4 个顶点与 2 个三角形。
///
/// 面的四个角按 uv (0,0)/(1,0)/(1,1)/(0,1) 的顺序排列，索引固定为
/// (0,1,2) 和 (0,2,3)，因此不需要关心顶点绕序（管线也关闭了背面剔除）。
fn push_face(mesh: &mut Mesh, x: i32, y: i32, z: i32, dir: [i32; 3], color: [f32; 3], tint: f32) {
    let axis = if dir[0] != 0 {
        0
    } else if dir[1] != 0 {
        1
    } else {
        2
    };
    let sign = dir[axis];
    let u_axis = (axis + 1) % 3;
    let v_axis = (axis + 2) % 3;

    let base = [x as f32, y as f32, z as f32];
    let corners: [(f32, f32, f32, f32); 4] = [
        (-1.0, -1.0, 0.0, 0.0),
        (1.0, -1.0, 1.0, 0.0),
        (1.0, 1.0, 1.0, 1.0),
        (-1.0, 1.0, 0.0, 1.0),
    ];

    let start = mesh.vertices.len() as u32;
    for (su, sv, u, v) in corners {
        let mut position = base;
        // 面贴在方块的边界上。
        position[axis] += if sign > 0 { 1.0 } else { 0.0 };
        position[u_axis] += 0.5 + 0.5 * su;
        position[v_axis] += 0.5 + 0.5 * sv;

        mesh.vertices.push(Vertex {
            position,
            normal: [dir[0] as f32, dir[1] as f32, dir[2] as f32],
            color: [
                color[0] * tint,
                color[1] * tint,
                color[2] * tint,
            ],
            uv: [u, v],
        });
    }

    mesh.indices
        .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}
