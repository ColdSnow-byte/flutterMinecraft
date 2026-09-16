// Flutter × Rust 渲染 demo 的冒烟测试。
//
// 测试环境下通常没有 rust_renderer.dll，页面会走"原生库不可用"的降级分支；
// 这里只验证界面可以正常构建、以及关键元素存在。

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:flutterrust/main.dart';

void main() {
  testWidgets('主界面可以正常构建', (WidgetTester tester) async {
    await tester.pumpWidget(const MyApp());

    expect(find.text('Flutter × Rust：Minecraft 风格体素 demo'), findsOneWidget);
    expect(find.text('渲染线程：Rust'), findsOneWidget);
    expect(find.text('停止'), findsOneWidget);

    // 卸载页面，避免测试结束时还有周期任务在跑。
    await tester.pumpWidget(const SizedBox());
  });
}
