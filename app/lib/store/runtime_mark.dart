import 'package:flutter/material.dart';

import '../sources/source_icons.dart' show MarkPainter;
import 'runtime_glyphs.dart';

/// A runtime's icon: the glyph its store names, on a rounded tile in the accent's container colour. A store that names none, or a
/// glyph the app does not draw, gets the generic chip icon.
class RuntimeMark extends StatelessWidget {
  const RuntimeMark({super.key, required this.icon, this.size = 48});

  /// The glyph's name, as the store's `icon` gives it.
  final String? icon;
  final double size;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final glyph = runtimeGlyphs[icon];
    return Container(
      key: const Key('runtime-mark'),
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(color: scheme.secondaryContainer, borderRadius: BorderRadius.circular(size / 5)),
      child: glyph == null
          ? Icon(Icons.memory, size: size * 0.56, color: scheme.onSecondaryContainer)
          : SizedBox.square(
              key: Key('runtime-glyph-$icon'),
              dimension: size * 0.58,
              child: CustomPaint(painter: MarkPainter(glyph, scheme.onSecondaryContainer)),
            ),
    );
  }
}
