import 'dart:async';

import 'package:flutter/material.dart';

import 'rust_renderer.dart';

void main() {
  runApp(const MyApp());
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Flutter + Rust Voxel',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        colorScheme: .fromSeed(seedColor: Colors.cyanAccent, brightness: Brightness.dark),
      ),
      home: const GamePage(),
    );
  }
}

class GamePage extends StatefulWidget {
  const GamePage({super.key});

  @override
  State<GamePage> createState() => _GamePageState();
}

class _GamePageState extends State<GamePage> {
  RustRenderer? _renderer;
  String? _failureMessage;

  int? _textureId;
  bool _playing = false;
  double _moveSpeed = 4.6;
  double _sensitivity = 0.0022;
  bool _colorSwap = false;
  String _nativeError = '';

  double _fps = 0;
  int _frameCount = 0;
  int _solidBlocks = 0;
  int _triangles = 0;
  ({double x, double y, double z})? _position;
  ({int x, int y, int z})? _selected;

  /// 最近一次交给 Rust 的渲染分辨率（物理像素）。
  Size _renderSize = Size.zero;
  Timer? _statsTimer;

  String get _fpsString => _fps.toStringAsFixed(1);

  @override
  void initState() {
    super.initState();
    try {
      _renderer = RustRenderer.instance;
    } on NativeRendererUnavailableException catch (e) {
      _failureMessage = e.message;
      return;
    }

    _statsTimer = Timer.periodic(const Duration(milliseconds: 250), (_) => _refresh());
    unawaited(_awaitTextureId());
  }

  @override
  void dispose() {
    _statsTimer?.cancel();
    _renderer?.setMouseLock(false);
    _renderer?.stop();
    super.dispose();
  }

  Future<void> _awaitTextureId() async {
    final renderer = _renderer!;
    final id = await renderer.waitForTextureId();
    if (!mounted) {
      return;
    }
    if (id == null) {
      setState(() => _failureMessage = '外部纹理注册超时，请检查 Windows 运行器是否已重新构建。');
      return;
    }
    setState(() => _textureId = id);
    _refresh();
  }

  /// 轮询原生侧状态（帧率、坐标、鼠标锁定、命中方块……）。
  void _refresh() {
    final renderer = _renderer;
    if (!mounted || renderer == null) {
      return;
    }
    setState(() {
      _fps = renderer.fps;
      _frameCount = renderer.frameCount;
      _solidBlocks = renderer.solidBlocks;
      _triangles = renderer.triangleCount;
      _position = renderer.playerPosition;
      _selected = renderer.selectedBlock;
      _playing = renderer.isMouseLocked;
      _nativeError = renderer.error;
    });
  }

  /// 把 Widget 的显示尺寸换算成物理像素交给 Rust 渲染器。
  void _updateRenderSize(BoxConstraints constraints) {
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final width = (constraints.maxWidth * dpr).round();
    final height = (constraints.maxHeight * dpr).round();
    if (width <= 0 || height <= 0) {
      return;
    }
    if (_renderSize.width == width && _renderSize.height == height) {
      return;
    }
    _renderSize = Size(width.toDouble(), height.toDouble());

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) {
        return;
      }
      if (_renderer!.isRunning) {
        _renderer!.resize(width, height);
      } else {
        _renderer!.start(width, height);
      }
    });
  }

  void _enterGame() {
    final renderer = _renderer;
    if (renderer == null || _textureId == null) {
      return;
    }
    // 锁定鼠标（ClipCursor + 隐藏光标），之后 WASD / 鼠标 / 左右键都归 Rust 处理。
    renderer.setMouseLock(true);
    Timer(const Duration(milliseconds: 120), _refresh);
  }

  void _leaveGame() {
    _renderer?.setMouseLock(false);
    _refresh();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Flutter × Rust：Minecraft 风格体素 demo'),
        backgroundColor: Theme.of(context).colorScheme.surface,
      ),
      body: Row(
        children: [
          Expanded(child: _buildStage()),
          const VerticalDivider(width: 1),
          SizedBox(width: 300, child: _buildControlPanel()),
        ],
      ),
    );
  }

  Widget _buildStage() {
    final failure = _failureMessage;
    return Padding(
      padding: const EdgeInsets.all(16),
      child: LayoutBuilder(
        builder: (context, constraints) {
          if (failure == null) {
            _updateRenderSize(constraints);
          }
          return ClipRRect(
            borderRadius: BorderRadius.circular(12),
            child: ColoredBox(
              color: const Color(0xFF0B0E14),
              child: _buildStageContent(failure),
            ),
          );
        },
      ),
    );
  }

  Widget _buildStageContent(String? failure) {
    if (failure != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text(
            failure,
            textAlign: TextAlign.center,
            style: const TextStyle(color: Colors.orangeAccent, fontSize: 15),
          ),
        ),
      );
    }

    final textureId = _textureId;
    if (textureId == null) {
      return const Center(child: CircularProgressIndicator());
    }

    return Stack(
      fit: StackFit.expand,
      children: [
        Texture(textureId: textureId),
        if (_playing) const Center(child: _Crosshair()),
        Positioned(left: 12, top: 12, child: _buildHud()),
        if (_playing)
          const Positioned(
            left: 0,
            right: 0,
            bottom: 12,
            child: Center(child: _Hint('Esc 退出鼠标锁定')),
          )
        else
          Positioned.fill(child: _buildStartOverlay()),
      ],
    );
  }

  Widget _buildHud() {
    final position = _position;
    final selected = _selected;
    final buffer = StringBuffer('$_fpsString FPS · $_frameCount 帧');
    if (position != null) {
      buffer.write(
        ' · XYZ ${position.x.toStringAsFixed(1)} '
        '${position.y.toStringAsFixed(1)} ${position.z.toStringAsFixed(1)}',
      );
    }
    if (selected != null) {
      buffer.write(' · 瞄准 (${selected.x}, ${selected.y}, ${selected.z})');
    }
    return _Badge(text: buffer.toString());
  }

  Widget _buildStartOverlay() {
    return GestureDetector(
      onTap: _enterGame,
      behavior: HitTestBehavior.opaque,
      child: Container(
        color: Colors.black.withValues(alpha: 0.45),
        child: const Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                '点击开始游戏',
                style: TextStyle(fontSize: 22, fontWeight: FontWeight.w700),
              ),
              SizedBox(height: 16),
              _KeyHint(keys: 'W A S D', label: '移动'),
              _KeyHint(keys: 'SPACE', label: '跳跃'),
              _KeyHint(keys: 'SHIFT', label: '疾跑'),
              _KeyHint(keys: '移动鼠标', label: '转视角'),
              _KeyHint(keys: '鼠标左键', label: '挖掉方块'),
              _KeyHint(keys: '鼠标右键', label: '放置方块'),
              _KeyHint(keys: 'ESC', label: '退出锁定'),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildControlPanel() {
    final renderer = _renderer;
    final ready = renderer != null && _textureId != null;
    final position = _position;
    final selected = _selected;

    return ListView(
      padding: const EdgeInsets.all(20),
      children: [
        const Text(
          '渲染线程：Rust',
          style: TextStyle(fontSize: 18, fontWeight: FontWeight.w600),
        ),
        const SizedBox(height: 4),
        const Text(
          'wgpu 渲染体素世界 -> 读回像素 -> Flutter 外部纹理。'
          '键盘鼠标由 Rust 直接轮询 Win32（WASD / 鼠标锁定 / 左右键）。',
          style: TextStyle(color: Colors.white60, height: 1.5),
        ),
        const SizedBox(height: 20),
        SizedBox(
          width: double.infinity,
          child: _playing
              ? OutlinedButton(
                  onPressed: _leaveGame,
                  child: const Text('退出鼠标锁定'),
                )
              : FilledButton(
                  onPressed: ready ? _enterGame : null,
                  child: const Text('开始游戏（锁定鼠标）'),
                ),
        ),
        const SizedBox(height: 12),
        Row(
          children: [
            Expanded(
              child: OutlinedButton(
                onPressed: ready
                    ? () {
                        renderer.start(
                          _renderSize.width.round(),
                          _renderSize.height.round(),
                        );
                      }
                    : null,
                child: const Text('恢复渲染'),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: OutlinedButton(
                onPressed: renderer == null
                    ? null
                    : () {
                        renderer.stop();
                        _refresh();
                      },
                child: const Text('停止'),
              ),
            ),
          ],
        ),
        const SizedBox(height: 16),
        Text('移动速度：${_moveSpeed.toStringAsFixed(1)} 方块/秒'),
        Slider(
          value: _moveSpeed,
          min: 1.5,
          max: 12,
          divisions: 21,
          onChanged: renderer == null
              ? null
              : (value) {
                  setState(() => _moveSpeed = value);
                  renderer.setMoveSpeed(value);
                },
        ),
        Text('鼠标灵敏度：${(_sensitivity * 1000).toStringAsFixed(2)}'),
        Slider(
          value: _sensitivity,
          min: 0.0005,
          max: 0.006,
          divisions: 22,
          onChanged: renderer == null
              ? null
              : (value) {
                  setState(() => _sensitivity = value);
                  renderer.setSensitivity(value);
                },
        ),
        SwitchListTile.adaptive(
          contentPadding: EdgeInsets.zero,
          value: _colorSwap,
          onChanged: renderer == null
              ? null
              : (value) {
                  setState(() => _colorSwap = value);
                  renderer.setColorSwap(value);
                },
          title: const Text('输出 BGRA（颜色反了就打开）'),
        ),
        const Divider(height: 32),
        _InfoRow(label: '帧率', value: '$_fpsString FPS'),
        _InfoRow(label: '鼠标锁定', value: _playing ? '已锁定' : '未锁定'),
        _InfoRow(label: '已渲染帧', value: '$_frameCount'),
        _InfoRow(
          label: '玩家坐标',
          value: position == null
              ? '-'
              : '${position.x.toStringAsFixed(2)}, '
                  '${position.y.toStringAsFixed(2)}, '
                  '${position.z.toStringAsFixed(2)}',
        ),
        _InfoRow(
          label: '瞄准方块',
          value: selected == null ? '-' : '${selected.x}, ${selected.y}, ${selected.z}',
        ),
        _InfoRow(label: '世界方块数', value: '$_solidBlocks'),
        _InfoRow(label: '三角形数', value: '$_triangles'),
        _InfoRow(
          label: '渲染分辨率',
          value: '${_renderSize.width.round()} × ${_renderSize.height.round()}',
        ),
        _InfoRow(label: '纹理 ID', value: '${_textureId ?? '-'}'),
        if (_nativeError.isNotEmpty) ...[
          const SizedBox(height: 16),
          Text(
            '原生错误：$_nativeError',
            style: const TextStyle(color: Colors.redAccent),
          ),
        ],
      ],
    );
  }
}

/// 屏幕中心的十字准星。
class _Crosshair extends StatelessWidget {
  const _Crosshair();

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: SizedBox(
        width: 18,
        height: 18,
        child: Stack(
          children: [
            Center(child: Container(width: 18, height: 2, color: Colors.white70)),
            Center(child: Container(width: 2, height: 18, color: Colors.white70)),
          ],
        ),
      ),
    );
  }
}

class _KeyHint extends StatelessWidget {
  const _KeyHint({required this.keys, required this.label});

  final String keys;
  final String label;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: RichText(
        text: TextSpan(
          style: const TextStyle(fontSize: 14, color: Colors.white),
          children: [
            TextSpan(
              text: '$keys  ',
              style: const TextStyle(
                fontWeight: FontWeight.w700,
                color: Colors.cyanAccent,
              ),
            ),
            TextSpan(text: label, style: const TextStyle(color: Colors.white70)),
          ],
        ),
      ),
    );
  }
}

class _Hint extends StatelessWidget {
  const _Hint(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    return _Badge(text: text);
  }
}

class _InfoRow extends StatelessWidget {
  const _InfoRow({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(label, style: const TextStyle(color: Colors.white60)),
          Text(value, style: const TextStyle(fontWeight: FontWeight.w600)),
        ],
      ),
    );
  }
}

class _Badge extends StatelessWidget {
  const _Badge({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: Colors.black.withValues(alpha: 0.55),
        borderRadius: BorderRadius.circular(6),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
        child: Text(
          text,
          style: const TextStyle(fontSize: 12, color: Colors.white70),
        ),
      ),
    );
  }
}
