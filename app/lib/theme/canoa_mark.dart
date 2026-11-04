import 'package:flutter/widgets.dart';

/// Canoa's mark: a Taíno dugout canoe with a paddle crossing it, drawn from the paths of
/// `assets/icons/canoa.svg` (a 64 by 64 box). Below [paddleFrom] logical pixels it is the hull alone, and
/// the spiral carved into the hull shows from [spiralFrom] up, as in the rendered icons.
class CanoaMark extends StatelessWidget {
  const CanoaMark({super.key, this.size = 24, required this.color});

  final double size;
  final Color color;

  /// The smallest size that draws the paddle; a 1 px shaft and its gap only blur the hull.
  static const double paddleFrom = 20;

  /// The smallest size that carves the spiral.
  static const double spiralFrom = 64;

  @override
  Widget build(BuildContext context) =>
      SizedBox.square(dimension: size, child: CustomPaint(painter: CanoaMarkPainter(color)));
}

/// Paints [CanoaMark] into whatever square it is given.
class CanoaMarkPainter extends CustomPainter {
  CanoaMarkPainter(this.color);

  final Color color;

  // The `d` and `stroke-width` attributes of canoa.svg, by id; canoa_mark_test.dart checks they match.
  static const hull = 'M 3.5 18 C 9 27 17 30 30 30 C 43 30 51 28 55 23 C 58 19 59 13 59.5 6 L 63.5 6 '
      'C 64.5 27 56 46 32 46 C 15 46 4 37 0 20 Z';
  static const cut = 'M 34.1 14.8 L 18.2 47.7 L 25 51 L 40.9 18.2 Z';
  static const spiral = 'M 46.6 36 C 46.7 36.3 46.6 36.9 46 37.1 C 45.4 37.3 44.5 36.9 44.3 36 '
      'C 44.1 35.1 44.8 33.9 46 33.7 C 47.2 33.5 48.7 34.5 48.9 36 C 49.1 37.5 47.8 39.3 46 39.4 '
      'C 44.2 39.6 42.2 38.1 42 36 C 41.8 33.9 43.6 31.6 46 31.4';
  static const spiralWidth = 1.3;
  static const paddle = 'M 43.3 0.2 L 20.9 46.4 C 19.4 49.6 16.4 48.2 14.8 51.4 C 12.3 56.6 14.7 61.3 15 63 '
      'C 16.5 62.2 21.7 61.1 24.2 56 C 25.8 52.7 22.8 51.3 24.4 48 L 46.7 1.8 Z';

  static final Path _hull = parse(hull), _cut = parse(cut), _spiral = parse(spiral), _paddle = parse(paddle);

  /// A path from absolute M, L, C and Z commands, the only ones canoa.svg uses.
  static Path parse(String d) {
    final tokens = RegExp(r'[A-Za-z]|-?\d+(?:\.\d+)?').allMatches(d).map((m) => m[0]!).toList();
    final path = Path();
    var i = 0;
    double next() => double.parse(tokens[i++]);
    while (i < tokens.length) {
      switch (tokens[i++]) {
        case 'M':
          path.moveTo(next(), next());
        case 'L':
          path.lineTo(next(), next());
        case 'C':
          path.cubicTo(next(), next(), next(), next(), next(), next());
        case 'Z':
          path.close();
        case final other:
          throw FormatException('Path command $other is not drawn', d);
      }
    }
    return path;
  }

  @override
  void paint(Canvas canvas, Size size) {
    final side = size.shortestSide;
    final paddled = side >= CanoaMark.paddleFrom;
    final fill = Paint()..color = color;
    final clear = Paint()..blendMode = BlendMode.clear;
    canvas
      ..save()
      ..translate((size.width - side) / 2, (size.height - side) / 2)
      ..scale(side / 64);
    // The hull and its cuts in a layer of their own, so clearing them leaves whatever is behind the mark.
    canvas
      ..saveLayer(const Rect.fromLTWH(0, 0, 64, 64), Paint())
      ..drawPath(_hull, fill);
    if (paddled) canvas.drawPath(_cut, clear);
    if (side >= CanoaMark.spiralFrom) {
      canvas.drawPath(_spiral, Paint()
        ..blendMode = BlendMode.clear
        ..style = PaintingStyle.stroke
        ..strokeWidth = spiralWidth
        ..strokeCap = StrokeCap.round);
    }
    canvas.restore();
    if (paddled) canvas.drawPath(_paddle, fill);
    canvas.restore();
  }

  @override
  bool shouldRepaint(CanoaMarkPainter oldDelegate) => oldDelegate.color != color;
}
