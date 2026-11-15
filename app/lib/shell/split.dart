import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'layout_state.dart';

/// A divider the person drags, or moves with the arrow keys once it has the focus, and resets with a double click.
///
/// It reports the width a drag asks for, not the width it has, so a bound is the owner's to apply. [vertical] false makes it a
/// horizontal bar that resizes a height.
class ResizeDivider extends StatefulWidget {
  const ResizeDivider({
    super.key,
    required this.label,
    required this.onDelta,
    required this.onEnd,
    required this.onReset,
    this.vertical = true,
    this.value,
  });

  /// What a screen reader calls it.
  final String label;

  /// The pixels a drag or a key moved the divider, positive away from the start edge of the region it sizes.
  final ValueChanged<double> onDelta;

  /// A drag or a key finished: the size is worth keeping.
  final VoidCallback onEnd;
  final VoidCallback onReset;
  final bool vertical;

  /// The size it stands for, for a screen reader.
  final String? value;

  @override
  State<ResizeDivider> createState() => _ResizeDividerState();
}

class _ResizeDividerState extends State<ResizeDivider> {
  bool _hover = false, _focus = false, _drag = false;
  final _node = FocusNode(debugLabel: 'divider');

  @override
  void dispose() {
    _node.dispose();
    super.dispose();
  }

  static const _step = 16.0, _bigStep = 64.0;

  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) return KeyEventResult.ignored;
    final big = HardwareKeyboard.instance.isShiftPressed;
    final step = big ? _bigStep : _step;
    final key = event.logicalKey;
    final back = widget.vertical ? LogicalKeyboardKey.arrowLeft : LogicalKeyboardKey.arrowUp;
    final forward = widget.vertical ? LogicalKeyboardKey.arrowRight : LogicalKeyboardKey.arrowDown;
    if (key == back) {
      widget.onDelta(-step);
    } else if (key == forward) {
      widget.onDelta(step);
    } else if (key == LogicalKeyboardKey.home) {
      widget.onReset();
      return KeyEventResult.handled;
    } else {
      return KeyEventResult.ignored;
    }
    widget.onEnd();
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final active = _hover || _focus || _drag;
    final line = Container(
      width: widget.vertical ? (active ? 3 : 1) : null,
      height: widget.vertical ? null : (active ? 3 : 1),
      color: active ? scheme.primary : scheme.outlineVariant,
    );
    return Semantics(
      container: true,
      label: widget.label,
      value: widget.value,
      hint: 'Drag, or use the arrow keys; Home resets',
      child: Focus(
        focusNode: _node,
        onKeyEvent: _key,
        onFocusChange: (focus) => setState(() => _focus = focus),
        child: MouseRegion(
          cursor: widget.vertical ? SystemMouseCursors.resizeColumn : SystemMouseCursors.resizeRow,
          onEnter: (_) => setState(() => _hover = true),
          onExit: (_) => setState(() => _hover = false),
          child: GestureDetector(
            key: ValueKey('divider ${widget.label}'),
            behavior: HitTestBehavior.opaque,
            dragStartBehavior: DragStartBehavior.down,
            onTapDown: (_) => _node.requestFocus(),
            onDoubleTap: widget.onReset,
            onHorizontalDragStart: widget.vertical ? (_) => setState(() => _drag = true) : null,
            onHorizontalDragUpdate: widget.vertical ? (details) => widget.onDelta(details.delta.dx) : null,
            onHorizontalDragEnd: widget.vertical ? (_) => _finish() : null,
            onVerticalDragStart: widget.vertical ? null : (_) => setState(() => _drag = true),
            onVerticalDragUpdate: widget.vertical ? null : (details) => widget.onDelta(details.delta.dy),
            onVerticalDragEnd: widget.vertical ? null : (_) => _finish(),
            child: SizedBox(
              width: widget.vertical ? 9 : null,
              height: widget.vertical ? null : 9,
              child: Center(child: line),
            ),
          ),
        ),
      ),
    );
  }

  void _finish() {
    setState(() => _drag = false);
    widget.onEnd();
  }
}

/// A section's index beside its main view, the divider between them, sized from [layout].
class IndexSplit extends StatelessWidget {
  const IndexSplit({super.key, required this.layout, required this.section, required this.fallbackWidth, required this.index, required this.main});

  final ShellLayout layout;
  final String section;

  /// The index's width until the person changes it.
  final double fallbackWidth;
  final Widget index, main;

  @override
  Widget build(BuildContext context) {
    final width = layout.indexWidth(section, fallbackWidth);
    return Row(children: [
      SizedBox(width: width, child: index),
      ResizeDivider(
        label: 'Resize the section index',
        value: '${width.round()} pixels',
        onDelta: (delta) => layout.setIndexWidth(section, layout.indexWidth(section, fallbackWidth) + delta),
        onEnd: () => layout.setIndexWidth(section, layout.indexWidth(section, fallbackWidth), keep: true),
        onReset: () => layout.resetIndexWidth(section),
      ),
      Expanded(child: main),
    ]);
  }
}
