# flutterrust

用 **Rust（wgpu + winit + glam）** 渲染，画面直接显示在 **Flutter 窗口**里的
Minecraft 风格体素 demo：WASD 移动、鼠标转视角、左键挖方块、右键放方块。

Flutter 桌面端（Windows）目前不支持 PlatformView，也不支持与外部渲染器共享 GPU
上下文，所以这里采用官方支持的 **外部纹理（external texture）** 方案：

```
winit 事件循环线程 (Rust)
        │  ① wgpu 把立方体渲染到离屏纹理
        │  ② copy_texture_to_buffer 同步读回 CPU 像素（RGBA8）
        ▼
   共享像素缓冲  ◄──── Mutex 保护的 Vec<u8>
        │  ③ 每帧回调 C++ -> TextureRegistrar::MarkTextureFrameAvailable()
        ▼
Windows 运行器 (C++)  ── flutter::PixelBufferTexture
        │  ④ Flutter 合成该帧时调用拷贝回调，memcpy 到 Flutter 的缓冲区
        ▼
Flutter 合成器 ──► Texture(textureId: ...) widget 显示
```

Dart 侧同时通过 `dart:ffi` 调用同一个动态库，用来启动/停止渲染、调整分辨率、
调节速度和切换通道顺序。

## 目录结构

| 路径 | 说明 |
| --- | --- |
| `rust/renderer/src/lib.rs` | C ABI、共享帧缓冲、winit 事件循环（帧节拍 + 跨线程命令） |
| `rust/renderer/src/renderer.rs` | wgpu 离屏渲染管线：立方体 + glam MVP + 深度测试 + 像素读回 |
| `rust/renderer/src/shader.wgsl` | WGSL 顶点/片元着色器（Lambert 光照） |
| `windows/runner/rust_surface_texture.{h,cpp}` | 把 Rust 像素缓冲包装成 Flutter 外部纹理 |
| `windows/runner/rust_renderer_ffi.h` | Rust C ABI 的 C++ 声明 |
| `lib/rust_renderer.dart` | Dart FFI 封装 |
| `lib/main.dart` | 界面：Texture 展示区 + 控制面板 |
| `windows/CMakeLists.txt` | 用 cargo 构建 Rust、拷贝 DLL、链接运行器 |

## 环境要求

- Flutter 3.19+（Windows desktop 已启用），本仓库在 Flutter 3.47 + Dart 3.13 上验证
- Rust stable（MSVC 工具链，`x86_64-pc-windows-msvc`）
- Visual Studio 2022（含「使用 C++ 的桌面开发」工作负载）

## 运行

```powershell
flutter pub get
flutter run -d windows          # 或者 flutter build windows
```

第一次构建时 CMake 会自动执行 `cargo build --release` 编译 `rust/renderer`，
并把 `rust_renderer.dll` 拷贝到 exe 同目录（Dart 的 `DynamicLibrary.open` 依赖它）。

只想单独编译 Rust 部分：

```powershell
cargo build --release --manifest-path rust/renderer/Cargo.toml
```

## 打包 MSIX（release）

`dart run msix:create`（配置见 pubspec.yaml 的 msix_config），
产物在 `build\windows\x64\runner\Release\flutterrust.msix`。
自签名证书需先信任（`--install-certificate`）才能双击安装。

## FFI 接口

| 函数 | 说明 |
| --- | --- |
| `rr_start(w, h)` / `rr_resize(w, h)` | 启动渲染线程 / 调整分辨率（单位：物理像素） |
| `rr_stop()` | 停止渲染线程 |
| `rr_set_speed(f32)` | 旋转速度倍率 |
| `rr_set_color_swap(bool)` | 输出通道顺序：`false` = RGBA，`true` = BGRA |
| `rr_texture_id()` | 外部纹理 ID，Dart 用它创建 `Texture` widget |
| `rr_copy_frame(dst, cap, w, h)` | Flutter 合成时拷贝最新像素 |
| `rr_fps()` / `rr_frame_count()` / `rr_last_error()` | 统计与错误信息 |

## 实现要点

- **winit 跑在非主线程**：需要在 Windows 上调用
  `EventLoopBuilderExtWindows::any_thread(true)`，否则 winit 会 panic。
- **winit 的两个用途**：`ControlFlow::WaitUntil` 作为帧节拍器，
  `EventLoopProxy::send_event` 作为 Flutter 线程 -> 渲染线程的安全命令通道。
- **帧率**：Windows 默认定时器精度是 15.6ms，直接限制到 ~40fps；Rust 侧临时调用
  `timeBeginPeriod(1)`，并用「帧开始时刻 + 16ms」作为截止时间，稳定在 60fps。
- **像素格式**：Flutter 外部纹理的像素格式随平台/后端而异。本 demo 默认输出
  RGBA（Impeller/ANGLE 下验证正确）；如果看到红蓝颠倒，打开界面上的
  **输出 BGRA** 开关即可。
- **性能**：每帧要走一次 GPU -> CPU -> GPU 的拷贝（1080p 约 8MB/帧），这是
  Windows 端外部纹理的固有成本；更高分辨率或更高帧率场景建议改成共享 D3D 纹理
  （`kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle`）。
