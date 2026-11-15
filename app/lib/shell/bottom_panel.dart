import 'package:flutter/material.dart';

import 'layout_state.dart';
import 'split.dart';

/// The bottom panel's frame: a divider that resizes it, and [child] (a work panel) in the height [layout] holds. It is drawn only
/// while a surface offers the bottom slot, so a window with none has no empty panel.
class BottomPanel extends StatelessWidget {
  const BottomPanel({super.key, required this.layout, required this.height, required this.maxHeight, required this.child});

  final ShellLayout layout;

  /// The height to draw, within what the window allows.
  final double height;

  /// The most the window allows, which a drag is held to.
  final double maxHeight;
  final Widget child;

  double _held(double value) => value.clamp(ShellLayout.bottomMin, maxHeight < ShellLayout.bottomMin ? ShellLayout.bottomMin : maxHeight);

  @override
  Widget build(BuildContext context) => SizedBox(
    height: height,
    child: Column(children: [
      ResizeDivider(
        label: 'Resize the bottom panel',
        value: '${height.round()} pixels',
        vertical: false,
        onDelta: (delta) => layout.setBottomHeight(_held(layout.bottomHeight) - delta),
        onEnd: () => layout.setBottomHeight(_held(layout.bottomHeight), keep: true),
        onReset: layout.resetBottomHeight,
      ),
      Expanded(child: child),
    ]),
  );
}
