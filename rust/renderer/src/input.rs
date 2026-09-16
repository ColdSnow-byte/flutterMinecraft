//! 输入处理。
//!
//! Flutter 桌面端没有"锁定指针 / 无限相对移动"的 API，所以键盘与鼠标都由 Rust
//! 侧直接轮询 Windows：键盘用 `GetAsyncKeyState`，鼠标用 `ClipCursor` 把光标限制在
//! 窗口客户区内、每帧读差分后把光标拽回中心（即 Minecraft 式的鼠标锁定）。

use glam::Vec2;

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInput {
    /// 前后 / 左右移动意图，取值 -1.0 / 0.0 / 1.0。
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sprint: bool,
    /// 本帧累计的鼠标位移（像素）。
    pub mouse_delta: Vec2,
    /// 本帧是否触发一次"挖方块"。
    pub break_block: bool,
    /// 本帧是否触发一次"放方块"。
    pub place_block: bool,
    /// 本帧是否按下了 Esc。
    pub escape: bool,
}

/// 按住鼠标时的连续触发间隔（秒）。
const REPEAT_INTERVAL: f32 = 0.25;

pub struct InputState {
    locked: bool,
    center: (i32, i32),
    prev_break_pressed: bool,
    prev_place_pressed: bool,
    break_cooldown: f32,
    place_cooldown: f32,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    pub fn new() -> Self {
        Self {
            locked: false,
            center: (0, 0),
            prev_break_pressed: false,
            prev_place_pressed: false,
            break_cooldown: 0.0,
            place_cooldown: 0.0,
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// 锁定鼠标：隐藏光标并把光标限制在窗口客户区内。
    ///
    /// 只有当前前台窗口属于本进程时才会成功，避免误锁其他程序。
    pub fn lock(&mut self) -> bool {
        #[cfg(target_os = "windows")]
        {
            let Some(area) = windows_impl::foreground_client_area() else {
                return false;
            };
            windows_impl::clip_cursor(Some(area));
            windows_impl::show_cursor(false);
            self.center = ((area.left + area.right) / 2, (area.top + area.bottom) / 2);
            windows_impl::set_cursor_pos(self.center);
            self.locked = true;
            true
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }

    /// 解锁鼠标（恢复光标显示与自由移动）。
    pub fn unlock(&mut self) {
        if !self.locked {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            windows_impl::clip_cursor(None);
            windows_impl::show_cursor(true);
        }
        self.locked = false;
    }

    /// 采样一帧输入。`dt` 用于计算按住鼠标的连续触发节奏。
    pub fn poll(&mut self, dt: f32) -> FrameInput {
        let mut input = FrameInput::default();

        self.break_cooldown = (self.break_cooldown - dt).max(0.0);
        self.place_cooldown = (self.place_cooldown - dt).max(0.0);

        #[cfg(target_os = "windows")]
        {
            if self.locked {
                // 鼠标：读差分并把光标拽回窗口中心，从而获得无限相对移动。
                if let Some((x, y)) = windows_impl::cursor_pos() {
                    let delta = Vec2::new((x - self.center.0) as f32, (y - self.center.1) as f32);
                    input.mouse_delta = delta;
                    if delta != Vec2::ZERO {
                        windows_impl::set_cursor_pos(self.center);
                    }
                }

                input.forward = key_down(windows_impl::VK_W) as i32 as f32
                    - key_down(windows_impl::VK_S) as i32 as f32;
                input.strafe = key_down(windows_impl::VK_D) as i32 as f32
                    - key_down(windows_impl::VK_A) as i32 as f32;
                input.jump = key_down(windows_impl::VK_SPACE);
                input.sprint = key_down(windows_impl::VK_SHIFT);
                input.escape = key_down(windows_impl::VK_ESCAPE);

                let break_pressed = key_down(windows_impl::VK_LBUTTON);
                if break_pressed && (!self.prev_break_pressed || self.break_cooldown <= 0.0) {
                    input.break_block = true;
                    self.break_cooldown = REPEAT_INTERVAL;
                }
                self.prev_break_pressed = break_pressed;

                let place_pressed = key_down(windows_impl::VK_RBUTTON);
                if place_pressed && (!self.prev_place_pressed || self.place_cooldown <= 0.0) {
                    input.place_block = true;
                    self.place_cooldown = REPEAT_INTERVAL;
                }
                self.prev_place_pressed = place_pressed;
            } else {
                // 未锁定时只关心 Esc 之外的按键会误触，因此完全不采样。
                self.prev_break_pressed = false;
                self.prev_place_pressed = false;
            }
        }

        input
    }

    /// 前台窗口已经不属于本进程（例如 Alt+Tab 切走了），需要自动解锁。
    pub fn should_auto_unlock(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.locked && !windows_impl::foreground_is_ours()
        }
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
    }
}

#[cfg(target_os = "windows")]
fn key_down(vk: i32) -> bool {
    unsafe { (windows_impl::GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::ffi::c_void;

    pub const VK_LBUTTON: i32 = 0x01;
    pub const VK_RBUTTON: i32 = 0x02;
    pub const VK_SHIFT: i32 = 0x10;
    pub const VK_ESCAPE: i32 = 0x1B;
    pub const VK_SPACE: i32 = 0x20;
    pub const VK_A: i32 = 0x41;
    pub const VK_D: i32 = 0x44;
    pub const VK_S: i32 = 0x53;
    pub const VK_W: i32 = 0x57;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn GetAsyncKeyState(vkey: i32) -> i16;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn SetCursorPos(x: i32, y: i32) -> i32;
        fn ClipCursor(rect: *const Rect) -> i32;
        fn ShowCursor(show: i32) -> i32;
        fn GetForegroundWindow() -> *mut c_void;
        fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
        fn GetClientRect(window: *mut c_void, rect: *mut Rect) -> i32;
        fn ClientToScreen(window: *mut c_void, point: *mut Point) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
    }

    fn foreground_window() -> *mut c_void {
        unsafe { GetForegroundWindow() }
    }

    pub fn foreground_is_ours() -> bool {
        let window = foreground_window();
        if window.is_null() {
            return false;
        }
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
            pid == GetCurrentProcessId()
        }
    }

    /// 前台窗口（本进程）客户区在屏幕上的矩形。
    pub fn foreground_client_area() -> Option<Rect> {
        if !foreground_is_ours() {
            return None;
        }
        let window = foreground_window();
        let mut rect = Rect::default();
        let mut origin = Point::default();
        unsafe {
            if GetClientRect(window, &mut rect) == 0 {
                return None;
            }
            if ClientToScreen(window, &mut origin) == 0 {
                return None;
            }
        }
        Some(Rect {
            left: origin.x,
            top: origin.y,
            right: origin.x + rect.right,
            bottom: origin.y + rect.bottom,
        })
    }

    pub fn cursor_pos() -> Option<(i32, i32)> {
        let mut point = Point::default();
        let ok = unsafe { GetCursorPos(&mut point) };
        (ok != 0).then_some((point.x, point.y))
    }

    pub fn set_cursor_pos((x, y): (i32, i32)) {
        unsafe {
            SetCursorPos(x, y);
        }
    }

    /// `None` 表示恢复光标的自由移动。
    pub fn clip_cursor(rect: Option<Rect>) {
        unsafe {
            match rect {
                Some(rect) => {
                    ClipCursor(&rect);
                }
                None => {
                    ClipCursor(std::ptr::null());
                }
            }
        }
    }

    pub fn show_cursor(show: bool) {
        unsafe {
            ShowCursor(i32::from(show));
        }
    }
}
