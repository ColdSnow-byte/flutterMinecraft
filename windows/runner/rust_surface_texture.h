#ifndef RUNNER_RUST_SURFACE_TEXTURE_H_
#define RUNNER_RUST_SURFACE_TEXTURE_H_

#include <flutter/texture_registrar.h>

#include <cstdint>
#include <memory>
#include <mutex>
#include <vector>

// 把 Rust 渲染线程产出的像素缓冲包装成一个 Flutter 外部纹理。
//
// 工作流程：
//   1. 构造时向引擎注册一个 flutter::PixelBufferTexture，拿到 texture_id；
//   2. 把"新帧就绪"回调交给 Rust 渲染线程；
//   3. Rust 每渲染完一帧就回调这里 -> MarkTextureFrameAvailable(texture_id)；
//   4. Flutter 合成该帧时调用 CopyPixelBuffer，把共享缓冲里的像素拷给 Flutter。
//
// 渲染线程和 Flutter 的 raster 线程会并发访问，因此这里用互斥量保护缓冲区；
// 每一帧的拷贝代价是一次 memcpy，这也是目前 Windows 端把外部内容放进 Flutter
// 窗口的标准做法。
class RustSurfaceTexture {
 public:
  explicit RustSurfaceTexture(flutter::TextureRegistrar* texture_registrar);
  ~RustSurfaceTexture();

  // 禁止拷贝：生命周期和 FlutterEngine 绑定。
  RustSurfaceTexture(const RustSurfaceTexture&) = delete;
  RustSurfaceTexture& operator=(const RustSurfaceTexture&) = delete;

  // Dart 侧通过 Texture widget 使用这个 ID。
  int64_t texture_id() const { return texture_id_; }

 private:
  // flutter::PixelBufferTexture 的拷贝回调，运行在 Flutter raster 线程。
  const FlutterDesktopPixelBuffer* CopyPixelBuffer(size_t width, size_t height);

  // Rust 渲染线程的回调入口。
  void NotifyFrameReady();
  static void NotifyFrameReadyThunk(void* user_data);

  flutter::TextureRegistrar* texture_registrar_;
  std::unique_ptr<flutter::TextureVariant> texture_variant_;
  int64_t texture_id_ = -1;

  std::mutex mutex_;
  std::vector<uint8_t> pixels_;
  FlutterDesktopPixelBuffer pixel_buffer_ = {};
};

#endif  // RUNNER_RUST_SURFACE_TEXTURE_H_
