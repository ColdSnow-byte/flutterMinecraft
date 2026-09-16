import 'dart:ffi' as ffi;
import 'dart:io';

// package:ffi 提供 Utf8 类型、toDartString() 扩展和 calloc。
import 'package:ffi/ffi.dart' as pkg_ffi;

/// "rust_renderer" 动态库缺失、或当前平台没有原生实现时的异常。
class NativeRendererUnavailableException implements Exception {
  NativeRendererUnavailableException(this.message);

  final String message;

  @override
  String toString() => message;
}

/// 对 Rust 渲染库（rust/renderer/src/lib.rs）的 FFI 封装。
///
/// 画面本身由 Windows 运行器注册的外部纹理负责送到 Flutter 合成器，
/// 因此这一层只负责"控制与状态"：启动 / 停止渲染、分辨率、鼠标锁定、
/// 灵敏度、移动速度，以及读取玩家坐标等统计信息。
class RustRenderer {
  RustRenderer._(this._lib) {
    _start = _lib.lookupFunction<_RRStart, _RRStartDart>('rr_start');
    _stop = _lib.lookupFunction<_RRStop, _RRStopDart>('rr_stop');
    _resize = _lib.lookupFunction<_RRResize, _RRResizeDart>('rr_resize');
    _setColorSwap =
        _lib.lookupFunction<_RRSetColorSwap, _RRSetColorSwapDart>('rr_set_color_swap');
    _setMouseLock =
        _lib.lookupFunction<_RRSetMouseLock, _RRSetMouseLockDart>('rr_set_mouse_lock');
    _isMouseLocked =
        _lib.lookupFunction<_RRIsMouseLocked, _RRIsMouseLockedDart>('rr_is_mouse_locked');
    _setSensitivity = _lib
        .lookupFunction<_RRSetSensitivity, _RRSetSensitivityDart>('rr_set_sensitivity');
    _setMoveSpeed =
        _lib.lookupFunction<_RRSetMoveSpeed, _RRSetMoveSpeedDart>('rr_set_move_speed');
    _playerPosition =
        _lib.lookupFunction<_RRPlayerPosition, _RRPlayerPositionDart>('rr_player_position');
    _selectedBlock =
        _lib.lookupFunction<_RRSelectedBlock, _RRSelectedBlockDart>('rr_selected_block');
    _solidBlocks =
        _lib.lookupFunction<_RRSolidBlocks, _RRSolidBlocksDart>('rr_solid_blocks');
    _triangleCount =
        _lib.lookupFunction<_RRTriangleCount, _RRTriangleCountDart>('rr_triangle_count');
    _textureId = _lib.lookupFunction<_RRTextureId, _RRTextureIdDart>('rr_texture_id');
    _frameCount = _lib.lookupFunction<_RRFrameCount, _RRFrameCountDart>('rr_frame_count');
    _fps = _lib.lookupFunction<_RRFps, _RRFpsDart>('rr_fps');
    _isRunning = _lib.lookupFunction<_RRIsRunning, _RRIsRunningDart>('rr_is_running');
    _lastError = _lib.lookupFunction<_RRLastError, _RRLastErrorDart>('rr_last_error');
  }

  static RustRenderer? _instance;

  /// 打开动态库并绑定符号。失败时抛出 [NativeRendererUnavailableException]。
  static RustRenderer get instance {
    final cached = _instance;
    if (cached != null) {
      return cached;
    }
    if (!Platform.isWindows) {
      throw NativeRendererUnavailableException(
        '这个 demo 的原生渲染库只在 Windows 构建中包含（当前平台：${Platform.operatingSystem}）。',
      );
    }
    try {
      // 运行器已经链接了这个 DLL，这里打开的是同一个模块实例。
      final lib = ffi.DynamicLibrary.open('rust_renderer.dll');
      _instance = RustRenderer._(lib);
      return _instance!;
    } on Object catch (e) {
      throw NativeRendererUnavailableException('加载 rust_renderer.dll 失败：$e');
    }
  }

  /// 是否已经有原生库可用（UI 用它决定要不要降级显示）。
  static bool get isAvailable {
    try {
      instance;
      return true;
    } on NativeRendererUnavailableException {
      return false;
    }
  }

  final ffi.DynamicLibrary _lib;

  late final _RRStartDart _start;
  late final _RRStopDart _stop;
  late final _RRResizeDart _resize;
  late final _RRSetColorSwapDart _setColorSwap;
  late final _RRSetMouseLockDart _setMouseLock;
  late final _RRIsMouseLockedDart _isMouseLocked;
  late final _RRSetSensitivityDart _setSensitivity;
  late final _RRSetMoveSpeedDart _setMoveSpeed;
  late final _RRPlayerPositionDart _playerPosition;
  late final _RRSelectedBlockDart _selectedBlock;
  late final _RRSolidBlocksDart _solidBlocks;
  late final _RRTriangleCountDart _triangleCount;
  late final _RRTextureIdDart _textureId;
  late final _RRFrameCountDart _frameCount;
  late final _RRFpsDart _fps;
  late final _RRIsRunningDart _isRunning;
  late final _RRLastErrorDart _lastError;

  /// 启动渲染线程（单位：物理像素）。重复调用只是更新分辨率。
  void start(int width, int height) {
    final result = _start(width, height);
    if (result != 0) {
      throw StateError('rr_start 失败（code=$result）：$error');
    }
  }

  void stop() => _stop();

  /// 调整渲染分辨率。
  void resize(int width, int height) => _resize(width, height);

  /// 设置输出通道顺序：`false` = RGBA，`true` = BGRA。
  /// 如果发现红蓝通道反了，把这个开关打开即可。
  void setColorSwap(bool swap) => _setColorSwap(swap);

  /// 锁定鼠标进入游戏操作模式（WASD + 鼠标转视角 + 左右键）。
  void setMouseLock(bool lock) => _setMouseLock(lock);

  /// 是否处于游戏操作模式（按 Esc 或切走窗口会自动退出）。
  bool get isMouseLocked => _isMouseLocked();

  /// 鼠标灵敏度：每像素对应的弧度。
  void setSensitivity(double value) => _setSensitivity(value);

  /// 移动速度：方块/秒。
  void setMoveSpeed(double value) => _setMoveSpeed(value);

  /// 玩家脚下坐标。
  ({double x, double y, double z})? get playerPosition {
    final buffer = pkg_ffi.calloc<ffi.Float>(3);
    try {
      final ok = _playerPosition(buffer, buffer + 1, buffer + 2);
      if (!ok) {
        return null;
      }
      return (x: buffer[0].toDouble(), y: buffer[1].toDouble(), z: buffer[2].toDouble());
    } finally {
      pkg_ffi.calloc.free(buffer);
    }
  }

  /// 视线命中的方块坐标。
  ({int x, int y, int z})? get selectedBlock {
    final buffer = pkg_ffi.calloc<ffi.Int32>(3);
    try {
      final ok = _selectedBlock(buffer, buffer + 1, buffer + 2);
      if (!ok) {
        return null;
      }
      return (x: buffer[0], y: buffer[1], z: buffer[2]);
    } finally {
      pkg_ffi.calloc.free(buffer);
    }
  }

  /// 世界里的方块总数。
  int get solidBlocks => _solidBlocks();

  /// 当前网格的三角形数量。
  int get triangleCount => _triangleCount();

  /// 外部纹理 ID；`-1` 表示原生纹理尚未注册完成。
  int get textureId => _textureId();

  /// 等待纹理 ID 就绪，超时返回 `null`。
  Future<int?> waitForTextureId({
    Duration timeout = const Duration(seconds: 10),
  }) async {
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      final id = textureId;
      if (id >= 0) {
        return id;
      }
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return null;
  }

  int get frameCount => _frameCount();

  double get fps => _fps();

  bool get isRunning => _isRunning();

  /// Rust 侧最后一条错误信息，无错误时为空字符串。
  String get error {
    final ptr = _lastError();
    if (ptr == ffi.nullptr) {
      return '';
    }
    return ptr.toDartString();
  }
}

// --- Native 侧函数类型 ---
typedef _RRStart = ffi.Int32 Function(ffi.Uint32, ffi.Uint32);
typedef _RRStop = ffi.Void Function();
typedef _RRResize = ffi.Int32 Function(ffi.Uint32, ffi.Uint32);
typedef _RRSetColorSwap = ffi.Void Function(ffi.Bool);
typedef _RRSetMouseLock = ffi.Void Function(ffi.Bool);
typedef _RRIsMouseLocked = ffi.Bool Function();
typedef _RRSetSensitivity = ffi.Void Function(ffi.Float);
typedef _RRSetMoveSpeed = ffi.Void Function(ffi.Float);
typedef _RRPlayerPosition = ffi.Bool Function(
    ffi.Pointer<ffi.Float>, ffi.Pointer<ffi.Float>, ffi.Pointer<ffi.Float>);
typedef _RRSelectedBlock = ffi.Bool Function(
    ffi.Pointer<ffi.Int32>, ffi.Pointer<ffi.Int32>, ffi.Pointer<ffi.Int32>);
typedef _RRSolidBlocks = ffi.Uint32 Function();
typedef _RRTriangleCount = ffi.Uint32 Function();
typedef _RRTextureId = ffi.Int64 Function();
typedef _RRFrameCount = ffi.Uint64 Function();
typedef _RRFps = ffi.Double Function();
typedef _RRIsRunning = ffi.Bool Function();
typedef _RRLastError = ffi.Pointer<pkg_ffi.Utf8> Function();

// --- Dart 侧函数类型 ---
typedef _RRStartDart = int Function(int, int);
typedef _RRStopDart = void Function();
typedef _RRResizeDart = int Function(int, int);
typedef _RRSetColorSwapDart = void Function(bool);
typedef _RRSetMouseLockDart = void Function(bool);
typedef _RRIsMouseLockedDart = bool Function();
typedef _RRSetSensitivityDart = void Function(double);
typedef _RRSetMoveSpeedDart = void Function(double);
typedef _RRPlayerPositionDart = bool Function(
    ffi.Pointer<ffi.Float>, ffi.Pointer<ffi.Float>, ffi.Pointer<ffi.Float>);
typedef _RRSelectedBlockDart = bool Function(
    ffi.Pointer<ffi.Int32>, ffi.Pointer<ffi.Int32>, ffi.Pointer<ffi.Int32>);
typedef _RRSolidBlocksDart = int Function();
typedef _RRTriangleCountDart = int Function();
typedef _RRTextureIdDart = int Function();
typedef _RRFrameCountDart = int Function();
typedef _RRFpsDart = double Function();
typedef _RRIsRunningDart = bool Function();
typedef _RRLastErrorDart = ffi.Pointer<pkg_ffi.Utf8> Function();
