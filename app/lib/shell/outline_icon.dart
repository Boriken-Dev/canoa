import 'package:flutter/material.dart';

/// The part of the window an outline icon stands for.
enum WindowRegion { left, bottom, right }

/// A small window drawn as an outline with one [region] filled: the symbol of a layout toggle. The region is filled with the
/// icon's colour while it is shown, and left empty while it is hidden.
class WindowOutline extends StatelessWidget {
  const WindowOutline({super.key, required this.region, required this.filled, this.size = 20, this.color});

  final WindowRegion region;
  final bool filled;
  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) => CustomPaint(
    size: Size.square(size),
    painter: _OutlinePainter(region, filled, color ?? IconTheme.of(context).color ?? Theme.of(context).colorScheme.onSurface),
  );
}

class _OutlinePainter extends CustomPainter {
  _OutlinePainter(this.region, this.filled, this.color);

  final WindowRegion region;
  final bool filled;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final stroke = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    final frame = Rect.fromLTWH(1.5, 3, size.width - 3, size.height - 6);
    final rounded = RRect.fromRectAndRadius(frame, const Radius.circular(2.5));
    final share = switch (region) {
      WindowRegion.left => Rect.fromLTWH(frame.left, frame.top, frame.width * 0.36, frame.height),
      WindowRegion.right => Rect.fromLTWH(frame.right - frame.width * 0.36, frame.top, frame.width * 0.36, frame.height),
      WindowRegion.bottom => Rect.fromLTWH(frame.left, frame.bottom - frame.height * 0.4, frame.width, frame.height * 0.4),
    };
    if (filled) {
      canvas.save();
      canvas.clipRRect(rounded);
      canvas.drawRect(share, Paint()..color = color);
      canvas.restore();
    } else {
      // The divider alone marks where the region is.
      final line = switch (region) {
        WindowRegion.left => (Offset(share.right, share.top), Offset(share.right, share.bottom)),
        WindowRegion.right => (Offset(share.left, share.top), Offset(share.left, share.bottom)),
        WindowRegion.bottom => (Offset(share.left, share.top), Offset(share.right, share.top)),
      };
      canvas.drawLine(line.$1, line.$2, stroke..strokeWidth = 1);
      stroke.strokeWidth = 1.5;
    }
    canvas.drawRRect(rounded, stroke);
  }

  @override
  bool shouldRepaint(_OutlinePainter old) => old.region != region || old.filled != filled || old.color != color;
}
