import 'package:material_ui/material_ui.dart';

/// One bar: [value] is null for a day nothing was recorded on, which draws no bar.
final class ChartBar {
  const ChartBar(this.label, this.value);

  final String label;
  final double? value;
}

/// A bar chart drawn with no package: one bar for each of [bars], scaled to the largest, with the largest value, the first and
/// the last label written. Whoever shows it shows the same numbers in a table beside it, so the chart never carries a figure alone.
class UsageBarChart extends StatelessWidget {
  const UsageBarChart({super.key, required this.bars, required this.format, this.height = 160, required this.description});

  final List<ChartBar> bars;

  /// Writes a value as the axis shows it.
  final String Function(double value) format;
  final double height;

  /// What a screen reader says of the chart.
  final String description;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final style = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant) ?? const TextStyle(fontSize: 12);
    final largest = bars.fold<double>(0, (a, b) => (b.value ?? 0) > a ? b.value! : a);
    return Semantics(
      label: description,
      child: ExcludeSemantics(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(largest > 0 ? format(largest) : '', style: style),
            const SizedBox(height: 4),
            SizedBox(
              height: height,
              child: CustomPaint(
                key: const Key('usage-chart'),
                painter: _BarPainter(bars: bars, bar: scheme.primary, axis: scheme.outlineVariant),
              ),
            ),
            const SizedBox(height: 4),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(bars.isEmpty ? '' : bars.first.label, style: style),
                Text(bars.length < 2 ? '' : bars.last.label, style: style),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _BarPainter extends CustomPainter {
  _BarPainter({required this.bars, required this.bar, required this.axis});

  final List<ChartBar> bars;
  final Color bar, axis;

  @override
  void paint(Canvas canvas, Size size) {
    final base = size.height - 1;
    canvas.drawLine(Offset(0, base), Offset(size.width, base), Paint()..color = axis);
    if (bars.isEmpty) return;
    final largest = bars.fold<double>(0, (a, b) => (b.value ?? 0) > a ? b.value! : a);
    if (largest <= 0) return;
    final slot = size.width / bars.length;
    final width = (slot * 0.7).clamp(1.0, 28.0);
    final paint = Paint()..color = bar;
    for (var i = 0; i < bars.length; i++) {
      final value = bars[i].value;
      if (value == null || value <= 0) continue;
      // A bar of any value shows at least two pixels, so a small day is not mistaken for none.
      final top = base - (base * value / largest).clamp(2.0, base);
      final left = slot * i + (slot - width) / 2;
      canvas.drawRect(Rect.fromLTRB(left, top, left + width, base), paint);
    }
  }

  @override
  bool shouldRepaint(_BarPainter old) => old.bars != bars || old.bar != bar || old.axis != axis;
}
