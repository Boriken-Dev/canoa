import 'package:flutter/material.dart';

import 'agent_marks.dart';
import 'sources.dart';
import 'svg_path.dart';

/// The generic icon for a provider with no vendor mark here: Grok (its vendor publishes none this app may use), any other
/// id, and an entry with no provider (a program on standard streams).
IconData providerIcon(String? harness) => switch (harness) {
  'grok' => Icons.bolt_outlined,
  _ => Icons.dns_outlined,
};

/// The provider id of [source]: a hosted entry's own, else the one a source of the host's names (Grok, Pi, Antigravity).
String? sourceProvider(AgentSource? source) =>
    source == null ? null : source.harness ?? (source.local ? source.label.toLowerCase() : null);

/// The generic icon of [source]: the terminal for a command a person wrote.
IconData sourceIcon(AgentSource? source) => source == null ? Icons.terminal : providerIcon(sourceProvider(source));

/// The vendor's mark for a provider, or the generic icon of [providerIcon] when it has none: from a hosted entry's
/// provider id ([SourceMark.provider]) or a whole source ([SourceMark.of]). A monochrome mark takes [color], else the
/// ambient icon colour.
class SourceMark extends StatelessWidget {
  const SourceMark.provider(this.provider, {super.key, this.size, this.color}) : source = null, _fromSource = false;
  const SourceMark.of(this.source, {super.key, this.size, this.color}) : provider = null, _fromSource = true;

  final String? provider;
  final AgentSource? source;
  final bool _fromSource;
  final double? size;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final theme = IconTheme.of(context);
    final side = size ?? theme.size ?? 24;
    final mark = agentMarks[_fromSource ? sourceProvider(source) : provider];
    if (mark == null) {
      return Icon(_fromSource ? sourceIcon(source) : providerIcon(provider), size: side, color: color);
    }
    return SizedBox.square(
      dimension: side,
      child: CustomPaint(painter: _MarkPainter(mark, color ?? theme.color ?? Theme.of(context).colorScheme.onSurface)),
    );
  }
}

class _MarkPainter extends CustomPainter {
  _MarkPainter(this.mark, this.color);

  final AgentMark mark;
  final Color color;

  static final _paths = <String, Path>{};

  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / mark.side, size.height / mark.side);
    for (final (d, fill) in mark.shapes) {
      canvas.drawPath(_paths.putIfAbsent(d, () => parseSvgPath(d)), Paint()..color = fill ?? color);
    }
  }

  @override
  bool shouldRepaint(_MarkPainter old) => old.mark != mark || old.color != color;
}
