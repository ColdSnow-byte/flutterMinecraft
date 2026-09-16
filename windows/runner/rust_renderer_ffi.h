#ifndef RUNNER_RUST_RENDERER_FFI_H_
#define RUNNER_RUST_RENDERER_FFI_H_

// rust/renderer/src/lib.rs 导出的 C ABI。
// 需要注意的是：这里的声明必须和 Rust 侧的 extern "C" 函数保持一致。

#include <cstddef>
#include <cstdint>

#ifdef _WIN32
#define RUST_API __declspec(dllimport)
#else
#define RUST_API
#endif

// Rust 侧每渲染完一帧都会调用它，参数是无类型的 user data。
typedef void (*rust_frame_ready_callback)(void* user_data);

extern "C" {
// 启动渲染线程（若已在运行则只更新分辨率），参数是物理像素尺寸。返回 0 表示成功。
RUST_API int32_t rr_start(uint32_t width, uint32_t height);
// 停止渲染线程。
RUST_API void rr_stop(void);
// 调整渲染分辨率。返回 0 表示成功。
RUST_API int32_t rr_resize(uint32_t width, uint32_t height);
// 输出通道顺序：false => RGBA，true => BGRA。
RUST_API void rr_set_color_swap(bool swap);
// 锁定 / 解锁鼠标（锁定后才能用 WASD + 鼠标操作）。
RUST_API void rr_set_mouse_lock(bool lock);
RUST_API bool rr_is_mouse_locked(void);
// 鼠标灵敏度（每像素弧度）与移动速度（方块/秒）。
RUST_API void rr_set_sensitivity(float sensitivity);
RUST_API void rr_set_move_speed(float speed);
// 玩家脚下坐标，返回是否读取成功。
RUST_API bool rr_player_position(float* x, float* y, float* z);
// 视线命中的方块坐标，返回是否有命中。
RUST_API bool rr_selected_block(int32_t* x, int32_t* y, int32_t* z);
RUST_API uint32_t rr_solid_blocks(void);
RUST_API uint32_t rr_triangle_count(void);
// 注册"新帧就绪"回调，传 nullptr 表示取消注册。
RUST_API void rr_set_frame_ready_callback(rust_frame_ready_callback callback,
                                          void* user_data);
// C++ 注册完外部纹理后把 ID 存到 Rust 侧，方便 Dart 统一查询。
RUST_API void rr_set_texture_id(int64_t id);
RUST_API int64_t rr_texture_id(void);
// 把最新的 RGBA 像素拷贝到 dst。返回 0 表示成功。
RUST_API int32_t rr_copy_frame(uint8_t* dst,
                               size_t dst_capacity,
                               uint32_t* out_width,
                               uint32_t* out_height);
// 当前帧需要的字节数（width * height * 4）。
RUST_API uint64_t rr_frame_byte_size(void);
RUST_API uint64_t rr_frame_count(void);
RUST_API double rr_fps(void);
RUST_API bool rr_is_running(void);
// 最后一条错误信息，无错误时返回空字符串（Rust 侧持有的静态字符串）。
RUST_API const char* rr_last_error(void);
}

#endif  // RUNNER_RUST_RENDERER_FFI_H_
