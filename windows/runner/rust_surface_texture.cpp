#include "rust_surface_texture.h"

#include "rust_renderer_ffi.h"

RustSurfaceTexture::RustSurfaceTexture(
    flutter::TextureRegistrar* texture_registrar)
    : texture_registrar_(texture_registrar) {
  if (texture_registrar_ == nullptr) {
    return;
  }

  auto copy_callback =
      [this](size_t width,
             size_t height) -> const FlutterDesktopPixelBuffer* {
    return this->CopyPixelBuffer(width, height);
  };

  texture_variant_ = std::make_unique<flutter::TextureVariant>(
      flutter::PixelBufferTexture(copy_callback));
  texture_id_ = texture_registrar_->RegisterTexture(texture_variant_.get());

  // 注册好纹理之后，把 ID 回传给 Rust，Dart 侧再通过 rr_texture_id() 取得；
  // 同时把新帧通知的回调交给渲染线程。
  rr_set_frame_ready_callback(&RustSurfaceTexture::NotifyFrameReadyThunk, this);
  rr_set_texture_id(texture_id_);
}

RustSurfaceTexture::~RustSurfaceTexture() {
  // 先摘掉回调再注销纹理，避免销毁过程中还有线程试图通知新帧。
  rr_set_frame_ready_callback(nullptr, nullptr);
  if (texture_registrar_ != nullptr && texture_id_ >= 0) {
    texture_registrar_->UnregisterTexture(texture_id_);
  }
}

const FlutterDesktopPixelBuffer* RustSurfaceTexture::CopyPixelBuffer(
    size_t width,
    size_t height) {
  // 真正显示多大由 Texture widget 决定，这里始终返回 Rust 渲染的原始分辨率。
  (void)width;
  (void)height;

  std::lock_guard<std::mutex> lock(mutex_);

  const uint64_t needed = rr_frame_byte_size();
  if (needed == 0) {
    return nullptr;  // 还没有可显示的帧。
  }
  if (pixels_.size() != static_cast<size_t>(needed)) {
    pixels_.resize(static_cast<size_t>(needed));
  }

  uint32_t frame_width = 0;
  uint32_t frame_height = 0;
  if (rr_copy_frame(pixels_.data(), pixels_.size(), &frame_width,
                    &frame_height) != 0) {
    return nullptr;
  }

  pixel_buffer_.buffer = pixels_.data();
  pixel_buffer_.width = frame_width;
  pixel_buffer_.height = frame_height;
  pixel_buffer_.release_callback = nullptr;
  pixel_buffer_.release_context = nullptr;
  return &pixel_buffer_;
}

void RustSurfaceTexture::NotifyFrameReady() {
  if (texture_registrar_ != nullptr && texture_id_ >= 0) {
    texture_registrar_->MarkTextureFrameAvailable(texture_id_);
  }
}

void RustSurfaceTexture::NotifyFrameReadyThunk(void* user_data) {
  auto* self = static_cast<RustSurfaceTexture*>(user_data);
  if (self != nullptr) {
    self->NotifyFrameReady();
  }
}
