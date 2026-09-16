//! 供 Flutter Windows 运行器通过 C ABI 调用的渲染服务。
//!
//! 这是一个 Minecraft 风格的体素 demo：Rust 侧用 wgpu 把体素世界渲染到离屏
//! 纹理，读回像素后由 Windows 运行器包装成 Flutter 外部纹理显示；键盘鼠标由
//! Rust 通过 Win32 API 直接轮询（WASD 移动、鼠标转视角、左键挖、右键放）。
//!
//! 数据流：
//!   winit 事件循环线程（更新世界 + wgpu 渲染 + 读回）
//!   -> 通知 C++ 侧 MarkTextureFrameAvailable
//!   -> Flutter 合成时回调 C++ 的像素拷贝函数，从共享缓冲拷贝到 Flutter 的纹理。

mod camera;
mod input;
mod renderer;
mod world;

use std::{
    ffi::{c_char, c_void, CString},
    sync::{
        atomic::Ordering,
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use glam::{IVec3, Vec2, Vec3};
use renderer::Renderer;
use winit::{
    application::ApplicationHandler,
    error::EventLoopError,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
};

use crate::{
    camera::Camera,
    input::InputState,
    world::{World, AIR, EYE_HEIGHT},
};

#[cfg(target_os = "windows")]
use winit::platform::windows::EventLoopBuilderExtWindows;

/// 目标帧率。
const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// 重力加速度（方块/秒²）。
const GRAVITY: f32 = 26.0;
/// 起跳初速度。
const JUMP_SPEED: f32 = 8.4;
const DEFAULT_MOVE_SPEED: f32 = 4.6;
const DEFAULT_SENSITIVITY: f32 = 0.0022;
/// 挖 / 放方块的最大距离。
const REACH: f32 = 6.0;

/// Flutter UI 线程 -> winit 渲染线程 的命令。
enum Command {
    Resize(u32, u32),
    SetColorSwap(bool),
    SetMouseLock(bool),
    SetSensitivity(f32),
    SetMoveSpeed(f32),
    Stop,
}

type FrameReadyCallback = extern "C" fn(*mut c_void);

/// 渲染结果与状态，由渲染线程写、Flutter 线程读。
struct SharedState {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    frames: u64,
    fps: f64,
    color_swap: bool,
    /// 新帧就绪回调（fn 指针 + usize 上下文，保证 Send）。
    callback: Option<FrameReadyCallback>,
    callback_ctx: usize,
    // --- 给 Flutter UI 读取的状态 ---
    mouse_locked: bool,
    sensitivity: f32,
    move_speed: f32,
    player: [f32; 3],
    selected: Option<[i32; 3]>,
    solid_blocks: u32,
    triangles: u32,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            frames: 0,
            fps: 0.0,
            color_swap: false,
            callback: None,
            callback_ctx: 0,
            mouse_locked: false,
            sensitivity: DEFAULT_SENSITIVITY,
            move_speed: DEFAULT_MOVE_SPEED,
            player: [0.0; 3],
            selected: None,
            solid_blocks: 0,
            triangles: 0,
        }
    }
}

static SHARED: OnceLock<Mutex<SharedState>> = OnceLock::new();
static LAST_ERROR: OnceLock<CString> = OnceLock::new();
/// Flutter 外部纹理 ID，由 C++ 侧注册后写入，Dart 读取。
static TEXTURE_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);
static PROXY: OnceLock<Mutex<Option<winit::event_loop::EventLoopProxy<Command>>>> =
    OnceLock::new();

fn shared() -> &'static Mutex<SharedState> {
    SHARED.get_or_init(Mutex::default)
}

fn proxy() -> &'static Mutex<Option<winit::event_loop::EventLoopProxy<Command>>> {
    PROXY.get_or_init(Mutex::default)
}

fn set_error(message: impl Into<String>) {
    // 跨语言读取错误信息，用 C 字符串（保证以 NUL 结尾）。只保留第一条错误。
    if let Ok(msg) = CString::new(message.into()) {
        let _ = LAST_ERROR.set(msg);
    }
}

fn is_running() -> bool {
    proxy().lock().map(|p| p.is_some()).unwrap_or(false)
}

fn dispatch(command: Command) -> bool {
    match proxy().lock() {
        Ok(guard) => match guard.as_ref() {
            Some(p) => p.send_event(command).is_ok(),
            None => false,
        },
        Err(_) => false,
    }
}

fn update_shared(mutate: impl FnOnce(&mut SharedState)) {
    if let Ok(mut state) = shared().lock() {
        mutate(&mut state);
    }
}

// --------------------------------------------------------------------------
// C ABI
// --------------------------------------------------------------------------

/// 启动渲染线程（如果尚未启动），并设置渲染分辨率（单位：物理像素）。
#[no_mangle]
pub extern "C" fn rr_start(width: u32, height: u32) -> i32 {
    // 已经在跑就只更新分辨率；如果命令发不出去（渲染线程正在退出），
    // 就继续往下走，重新起一个线程。
    if is_running() && dispatch(Command::Resize(width, height)) {
        return 0;
    }

    std::thread::Builder::new()
        .name("rust-render-thread".into())
        .spawn(move || run_event_loop(width, height))
        .map(|_| 0)
        .unwrap_or_else(|e| {
            set_error(format!("启动渲染线程失败: {e}"));
            -1
        })
}

/// 停止渲染线程。
#[no_mangle]
pub extern "C" fn rr_stop() {
    dispatch(Command::Stop);
}

/// 调整渲染分辨率。
#[no_mangle]
pub extern "C" fn rr_resize(width: u32, height: u32) -> i32 {
    i32::from(!dispatch(Command::Resize(width, height)))
}

/// 选择输出通道顺序：false = RGBA，true = BGRA。
#[no_mangle]
pub extern "C" fn rr_set_color_swap(swap: bool) {
    update_shared(|state| state.color_swap = swap);
    dispatch(Command::SetColorSwap(swap));
}

/// 锁定 / 解锁鼠标（锁定后才能用 WASD + 鼠标操作）。
#[no_mangle]
pub extern "C" fn rr_set_mouse_lock(lock: bool) {
    dispatch(Command::SetMouseLock(lock));
}

/// 鼠标是否处于锁定（游戏中）状态。
#[no_mangle]
pub extern "C" fn rr_is_mouse_locked() -> bool {
    shared().lock().map(|s| s.mouse_locked).unwrap_or(false)
}

/// 鼠标灵敏度（每像素对应的弧度）。
#[no_mangle]
pub extern "C" fn rr_set_sensitivity(sensitivity: f32) {
    update_shared(|state| state.sensitivity = sensitivity);
    dispatch(Command::SetSensitivity(sensitivity));
}

/// 移动速度（方块/秒）。
#[no_mangle]
pub extern "C" fn rr_set_move_speed(speed: f32) {
    update_shared(|state| state.move_speed = speed);
    dispatch(Command::SetMoveSpeed(speed));
}

/// 玩家脚下位置。
#[no_mangle]
pub extern "C" fn rr_player_position(x: *mut f32, y: *mut f32, z: *mut f32) -> bool {
    if x.is_null() || y.is_null() || z.is_null() {
        return false;
    }
    let Ok(state) = shared().lock() else {
        return false;
    };
    unsafe {
        *x = state.player[0];
        *y = state.player[1];
        *z = state.player[2];
    }
    true
}

/// 视线命中的方块坐标，返回是否有命中。
#[no_mangle]
pub extern "C" fn rr_selected_block(x: *mut i32, y: *mut i32, z: *mut i32) -> bool {
    if x.is_null() || y.is_null() || z.is_null() {
        return false;
    }
    let Ok(state) = shared().lock() else {
        return false;
    };
    match state.selected {
        Some(block) => {
            unsafe {
                *x = block[0];
                *y = block[1];
                *z = block[2];
            }
            true
        }
        None => false,
    }
}

/// 世界里的方块总数。
#[no_mangle]
pub extern "C" fn rr_solid_blocks() -> u32 {
    shared().lock().map(|s| s.solid_blocks).unwrap_or(0)
}

/// 当前网格的三角形数量。
#[no_mangle]
pub extern "C" fn rr_triangle_count() -> u32 {
    shared().lock().map(|s| s.triangles).unwrap_or(0)
}

/// 设置"新帧就绪"回调，C++ 侧会用它调用 MarkTextureFrameAvailable。
#[no_mangle]
pub extern "C" fn rr_set_frame_ready_callback(
    callback: Option<FrameReadyCallback>,
    ctx: *mut c_void,
) {
    let mut state = match shared().lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    state.callback = callback;
    state.callback_ctx = ctx as usize;
}

/// C++ 侧注册外部纹理成功后写入 ID，Dart 侧读取。
#[no_mangle]
pub extern "C" fn rr_set_texture_id(id: i64) {
    TEXTURE_ID.store(id, Ordering::SeqCst);
}

/// Dart 侧读取纹理 ID，-1 表示尚未注册。
#[no_mangle]
pub extern "C" fn rr_texture_id() -> i64 {
    TEXTURE_ID.load(Ordering::SeqCst)
}

/// Flutter 合成时调用：把最新的 RGBA 像素拷贝到 Flutter 提供的缓冲区。
///
/// 返回 0 表示成功，-1 参数非法，-2 缓冲区过小，-3 还没有可用帧。
#[no_mangle]
pub extern "C" fn rr_copy_frame(
    dst: *mut u8,
    dst_capacity: usize,
    out_width: *mut u32,
    out_height: *mut u32,
) -> i32 {
    if dst.is_null() || out_width.is_null() || out_height.is_null() {
        return -1;
    }

    let state = match shared().lock() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    if state.pixels.is_empty() {
        return -3;
    }
    if dst_capacity < state.pixels.len() {
        return -2;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(state.pixels.as_ptr(), dst, state.pixels.len());
        *out_width = state.width;
        *out_height = state.height;
    }
    0
}

/// 当前缓冲区大小所需的字节数，C++ 侧用它申请缓冲区。
#[no_mangle]
pub extern "C" fn rr_frame_byte_size() -> u64 {
    let state = match shared().lock() {
        Ok(s) => s,
        Err(_) => return 0,
    };
    state.pixels.len() as u64
}

/// 已渲染帧数。
#[no_mangle]
pub extern "C" fn rr_frame_count() -> u64 {
    shared().lock().map(|s| s.frames).unwrap_or(0)
}

/// 最近的平均帧率。
#[no_mangle]
pub extern "C" fn rr_fps() -> f64 {
    shared().lock().map(|s| s.fps).unwrap_or(0.0)
}

/// 渲染是否在运行。
#[no_mangle]
pub extern "C" fn rr_is_running() -> bool {
    is_running()
}

/// 最后一条错误信息（无错误时返回空字符串）。
#[no_mangle]
pub extern "C" fn rr_last_error() -> *const c_char {
    match LAST_ERROR.get() {
        Some(msg) => msg.as_ptr().cast::<c_char>(),
        None => c"".as_ptr(),
    }
}

// --------------------------------------------------------------------------
// winit 事件循环
// --------------------------------------------------------------------------

/// winit 默认要求事件循环跑在主线程上，但主线程归 Flutter 所有，所以这里显式
/// 允许在任意线程创建（Windows 平台专属开关）。
fn build_event_loop() -> Result<EventLoop<Command>, EventLoopError> {
    let mut builder = EventLoop::<Command>::with_user_event();
    #[cfg(target_os = "windows")]
    builder.with_any_thread(true);
    builder.build()
}

/// Windows 默认的定时器精度是 15.6ms，会直接把 ControlFlow::WaitUntil 的
/// 帧间隔拉到 ~25ms。这里临时把精度提到 1ms，才能稳定跑到 60fps。
#[cfg(target_os = "windows")]
mod timer_resolution {
    #[link(name = "winmm")]
    extern "system" {
        fn timeBeginPeriod(period: u32) -> u32;
        fn timeEndPeriod(period: u32) -> u32;
    }

    pub fn acquire() {
        unsafe {
            timeBeginPeriod(1);
        }
    }

    pub fn release() {
        unsafe {
            timeEndPeriod(1);
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod timer_resolution {
    pub fn acquire() {}
    pub fn release() {}
}

fn run_event_loop(width: u32, height: u32) {
    let event_loop = match build_event_loop() {
        Ok(el) => el,
        Err(e) => {
            set_error(format!("创建 winit 事件循环失败: {e}"));
            return;
        }
    };

    if let Ok(mut guard) = proxy().lock() {
        *guard = Some(event_loop.create_proxy());
    }

    let mut app = RenderApp::new(width, height);

    timer_resolution::acquire();
    let result = event_loop.run_app(&mut app);
    timer_resolution::release();

    if let Err(e) = result {
        set_error(format!("winit 事件循环异常: {e}"));
    }

    if let Ok(mut guard) = proxy().lock() {
        *guard = None;
    }
}

struct RenderApp {
    size: (u32, u32),
    renderer: Option<Renderer>,
    world: Option<World>,
    camera: Camera,
    /// 玩家脚部中心位置。
    player_position: Vec3,
    velocity_y: f32,
    on_ground: bool,
    input: InputState,
    sensitivity: f32,
    move_speed: f32,
    /// 网格对应的世界版本号，用来决定是否重建网格。
    mesh_revision: u64,
    selected: Option<IVec3>,
    /// 复用一块内存做 readback 输出，避免每帧分配。
    scratch: Vec<u8>,
    last_tick: Instant,
    fps: f64,
}

impl RenderApp {
    fn new(width: u32, height: u32) -> Self {
        let (sensitivity, move_speed) = shared()
            .lock()
            .map(|s| (s.sensitivity, s.move_speed))
            .unwrap_or((DEFAULT_SENSITIVITY, DEFAULT_MOVE_SPEED));

        Self {
            size: (width, height),
            renderer: None,
            world: None,
            camera: Camera::new(Vec3::ZERO),
            player_position: Vec3::ZERO,
            velocity_y: 0.0,
            on_ground: false,
            input: InputState::new(),
            sensitivity,
            move_speed,
            mesh_revision: u64::MAX,
            selected: None,
            scratch: Vec::new(),
            last_tick: Instant::now(),
            fps: 0.0,
        }
    }

    fn ensure_world(&mut self, event_loop: &ActiveEventLoop) {
        if self.world.is_some() {
            return;
        }

        // 生成地形并把玩家放到平坦出生点的地面上。
        let world = World::generate();
        let (spawn_x, spawn_z) = world.spawn;
        let spawn_y = world.surface_height(spawn_x, spawn_z) as f32 + 1.0;

        self.player_position = Vec3::new(spawn_x as f32 + 0.5, spawn_y, spawn_z as f32 + 0.5);
        self.camera = Camera::new(self.player_position + Vec3::new(0.0, EYE_HEIGHT, 0.0));
        self.world = Some(world);

        if self.renderer.is_none() {
            match Renderer::new(self.size.0, self.size.1) {
                Ok(renderer) => self.renderer = Some(renderer),
                Err(e) => {
                    set_error(e);
                    event_loop.exit();
                }
            }
        }
    }

    /// 摄像机与玩家物理。
    fn update_player(&mut self, forward: f32, strafe: f32, jump: bool, sprint: bool, dt: f32) {
        let speed = if sprint {
            self.move_speed * 1.7
        } else {
            self.move_speed
        };

        let wish = (self.camera.forward_horizontal() * forward + self.camera.right() * strafe)
            .normalize_or_zero()
            * speed;

        let Some(world) = self.world.as_ref() else {
            return;
        };

        let mut position = self.player_position;

        // 分轴移动，撞到方块就把该轴的位移回退（简单的 AABB 碰撞）。
        let delta_x = wish.x * dt;
        position.x += delta_x;
        if world.collides(position) {
            position.x -= delta_x;
        }

        let delta_z = wish.z * dt;
        position.z += delta_z;
        if world.collides(position) {
            position.z -= delta_z;
        }

        // 重力与跳跃。
        self.velocity_y -= GRAVITY * dt;
        if jump && self.on_ground {
            self.velocity_y = JUMP_SPEED;
        }

        let delta_y = self.velocity_y * dt;
        position.y += delta_y;
        if world.collides(position) {
            position.y -= delta_y;
            if self.velocity_y < 0.0 {
                self.on_ground = true;
            }
            self.velocity_y = 0.0;
        } else {
            self.on_ground = false;
        }

        // 贴着地面走时也算"站在地上"，这样随时都能跳。
        if !self.on_ground && world.collides(position + Vec3::new(0.0, -0.03, 0.0)) {
            self.on_ground = true;
            if self.velocity_y < 0.0 {
                self.velocity_y = 0.0;
            }
        }

        self.player_position = position;
    }

    fn break_block(&mut self, block: IVec3) {
        if let Some(world) = self.world.as_mut() {
            world.set(block.x, block.y, block.z, AIR);
        }
    }

    fn place_block(&mut self, hit: world::RayHit) {
        let target = hit.block + IVec3::new(hit.face[0], hit.face[1], hit.face[2]);

        // 不要把方块放进玩家自己身上。
        let min = Vec3::new(
            target.x as f32,
            target.y as f32,
            target.z as f32,
        );
        let max = min + Vec3::splat(1.0);
        let player_min = Vec3::new(
            self.player_position.x - world::PLAYER_HALF_WIDTH,
            self.player_position.y,
            self.player_position.z - world::PLAYER_HALF_WIDTH,
        );
        let player_max = player_min + Vec3::new(0.6, world::PLAYER_HEIGHT, 0.6);
        let overlapping = min.x < player_max.x
            && max.x > player_min.x
            && min.y < player_max.y
            && max.y > player_min.y
            && min.z < player_max.z
            && max.z > player_min.z;
        if overlapping {
            return;
        }

        let Some(world) = self.world.as_mut() else {
            return;
        };
        // 放置的方块沿用被点击方块的类型，手感上更像 Minecraft。
        let block_id = world.get(hit.block.x, hit.block.y, hit.block.z).max(1);
        world.set(target.x, target.y, target.z, block_id);
    }

    /// 世界发生修改时重建并上传网格。
    fn sync_mesh(&mut self) {
        let Some(world) = self.world.as_ref() else {
            return;
        };
        if world.revision == self.mesh_revision {
            return;
        }

        let mesh = world.build_mesh();
        let solid_blocks = world.solid_count;
        self.mesh_revision = world.revision;

        let triangles = match self.renderer.as_mut() {
            Some(renderer) => {
                renderer.upload_mesh(&mesh.vertices, &mesh.indices);
                renderer.triangle_count
            }
            None => 0,
        };

        update_shared(|state| {
            state.solid_blocks = solid_blocks;
            state.triangles = triangles;
        });
    }

    fn render(&mut self) {
        let (width, height) = match self.renderer.as_ref() {
            Some(renderer) => renderer.size(),
            None => return,
        };
        let aspect = width as f32 / height as f32;
        let mvp = self.camera.projection_matrix(aspect) * self.camera.view_matrix();
        let selection = self.selected;
        let color_swap = shared().lock().map(|s| s.color_swap).unwrap_or(false);

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        renderer.render_frame(mvp, selection, color_swap, &mut self.scratch);
        let (width, height) = renderer.size();

        let mut state = match shared().lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        state.pixels.clear();
        state.pixels.extend_from_slice(&self.scratch);
        state.width = width;
        state.height = height;
        state.frames += 1;
        state.fps = self.fps;
        state.player = [
            self.player_position.x,
            self.player_position.y,
            self.player_position.z,
        ];
        state.selected = selection.map(|b| [b.x, b.y, b.z]);
        state.mouse_locked = self.input.is_locked();

        if let Some(callback) = state.callback {
            let ctx = state.callback_ctx as *mut c_void;
            drop(state);
            callback(ctx);
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_tick)
            .as_secs_f32()
            .min(0.05);
        self.last_tick = now;

        if dt > 0.0 {
            let instant_fps = 1.0 / dt as f64;
            self.fps = if self.fps == 0.0 {
                instant_fps
            } else {
                self.fps * 0.9 + instant_fps * 0.1
            };
        }

        if self.renderer.is_none() {
            return;
        }

        // ---- 输入 ----
        let frame = self.input.poll(dt);
        if frame.escape && self.input.is_locked() {
            self.input.unlock();
        }
        if self.input.should_auto_unlock() {
            // 例如 Alt+Tab 切到别的程序，自动退出鼠标锁定，避免光标被锁在窗口里。
            self.input.unlock();
        }

        if self.input.is_locked() {
            if frame.mouse_delta != Vec2::ZERO {
                self.camera
                    .rotate(frame.mouse_delta.x, frame.mouse_delta.y, self.sensitivity);
            }
            self.update_player(
                frame.forward,
                frame.strafe,
                frame.jump,
                frame.sprint,
                dt,
            );
        } else {
            self.velocity_y = 0.0;
        }

        // 摄像机跟随玩家眼睛。
        self.camera.position = self.player_position + Vec3::new(0.0, EYE_HEIGHT, 0.0);

        // ---- 拾取与交互 ----
        let hit = self.world.as_ref().and_then(|world| {
            world.raycast(self.camera.position, self.camera.forward(), REACH)
        });
        self.selected = hit.map(|h| h.block);

        if self.input.is_locked() {
            if let Some(hit) = hit {
                if frame.break_block {
                    self.break_block(hit.block);
                } else if frame.place_block {
                    self.place_block(hit);
                }
            }
        }

        self.sync_mesh();
        self.render();
    }
}

impl ApplicationHandler<Command> for RenderApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.ensure_world(event_loop);
        self.last_tick = Instant::now();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
        // 渲染完全离屏，没有自己的窗口，因此没有窗口事件需要处理。
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Command) {
        match event {
            Command::Resize(width, height) => {
                self.size = (width.max(1), height.max(1));
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(width.max(1), height.max(1));
                }
            }
            Command::SetColorSwap(swap) => {
                update_shared(|state| state.color_swap = swap);
            }
            Command::SetMouseLock(lock) => {
                if lock {
                    // 只有窗口在前台时才可能锁定成功（否则会去锁别的程序）。
                    // 失败不写全局错误，UI 通过 is_mouse_locked 就能看出来。
                    if !self.input.lock() {
                        eprintln!("[rust_renderer] 鼠标锁定失败：窗口不在前台");
                    }
                } else {
                    self.input.unlock();
                }
                update_shared(|state| state.mouse_locked = self.input.is_locked());
            }
            Command::SetSensitivity(sensitivity) => {
                self.sensitivity = sensitivity;
                update_shared(|state| state.sensitivity = sensitivity);
            }
            Command::SetMoveSpeed(speed) => {
                self.move_speed = speed;
                update_shared(|state| state.move_speed = speed);
            }
            Command::Stop => event_loop.exit(),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 以"帧开始时刻 + 16ms"作为截止时间，这样渲染本身的耗时不会被累加到
        // 帧间隔里（否则同步读回的开销会直接吃掉帧率）。
        let frame_start = Instant::now();
        self.tick();
        let deadline = frame_start + FRAME_INTERVAL;
        let now = Instant::now();
        event_loop.set_control_flow(ControlFlow::WaitUntil(if deadline > now {
            deadline
        } else {
            now
        }));
    }
}

impl Drop for RenderApp {
    fn drop(&mut self) {
        // 别把光标锁在窗口里。
        self.input.unlock();
    }
}
