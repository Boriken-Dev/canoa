import 'package:flutter/widgets.dart';

/// Canoa's mark: a Taíno dugout canoe with a paddle crossing it, drawn from the paths of
/// `assets/icons/canoa.svg` (a 64 by 64 box). Below [paddleFrom] logical pixels it is the hull alone, stroked
/// [CanoaMarkPainter.smallWeight] units wide so its long, low shape keeps some depth, and the spiral carved
/// into the hull shows from [spiralFrom] up, as in the rendered icons.
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
  static const hull = 'M 2 23 C 5 27 9 30 15 30 L 49 30 C 55 30 59 27 60.5 17 L 63.5 17 '
      'C 64.5 31 59 41 49 41.5 C 38 42.5 26 42.5 15 41.5 C 6 40.5 1 32 0 24 Z';
  static const smallWeight = 3.0;
  static const cut = 'M 34.1 14.8 L 19.2 45.5 L 26.1 48.8 L 40.9 18.2 Z';
  static const spiral = 'M 47.1 35.9 C 47.3 36.2 47.1 36.9 46.5 37.1 C 45.9 37.4 44.8 36.9 44.6 35.9 '
      'C 44.4 34.9 45.2 33.5 46.5 33.3 C 47.8 33.1 49.5 34.2 49.8 35.9 C 50 37.6 48.6 39.6 46.5 39.8 '
      'C 44.4 40.1 42.1 38.3 41.9 35.9';
  static const spiralWidth = 1.3;
  static const paddle = 'M 43.3 0.2 L 21.8 44.5 C 20 48.4 16.6 46.8 14.8 50.6 C 12.1 56.1 14.7 61.3 15 63 '
      'C 16.5 62.2 22.2 61 24.8 55.5 C 26.7 51.6 23.4 50 25.3 46.2 L 46.7 1.8 Z';

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
    if (paddled) {
      canvas.drawPath(_cut, clear);
    } else {
      canvas.drawPath(_hull, Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = smallWeight
        ..strokeJoin = StrokeJoin.round);
    }
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
