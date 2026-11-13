import 'dart:math' as math;

import 'package:flutter/painting.dart';

/// A [Path] from SVG path data: M, L, H, V, C, S, A and Z, absolute and relative, the commands the vendors' marks use.
/// Any other command is refused rather than drawn wrong.
Path parseSvgPath(String d) {
  final tokens = RegExp(r'[A-Za-z]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?').allMatches(d).map((m) => m[0]!).toList();
  final path = Path();
  var i = 0;
  var x = 0.0, y = 0.0, startX = 0.0, startY = 0.0;
  Offset? lastControl;
  String? command;

  bool isCommand(String t) => RegExp(r'^[A-Za-z]$').hasMatch(t);
  double number() => double.parse(tokens[i++]);
  // An arc's two flags may be written with no separator from what follows them: "0 01.5" is flag 0, flag 1, then .5.
  double flag() {
    final t = tokens[i];
    if (t == '0' || t == '1') return number();
    tokens[i] = t.substring(1);
    return double.parse(t[0]);
  }

  while (i < tokens.length) {
    if (isCommand(tokens[i])) command = tokens[i++];
    final c = command;
    if (c == null) throw FormatException('Path data starts with a number', d);
    final rel = c == c.toLowerCase();
    final dx = rel ? x : 0.0, dy = rel ? y : 0.0;
    Offset? control;
    switch (c.toUpperCase()) {
      case 'M':
        x = number() + dx;
        y = number() + dy;
        startX = x;
        startY = y;
        path.moveTo(x, y);
        command = rel ? 'l' : 'L';
      case 'L':
        x = number() + dx;
        y = number() + dy;
        path.lineTo(x, y);
      case 'H':
        x = number() + dx;
        path.lineTo(x, y);
      case 'V':
        y = number() + dy;
        path.lineTo(x, y);
      case 'C':
        final x1 = number() + dx, y1 = number() + dy, x2 = number() + dx, y2 = number() + dy;
        x = number() + dx;
        y = number() + dy;
        path.cubicTo(x1, y1, x2, y2, x, y);
        control = Offset(x2, y2);
      case 'S':
        final reflected = lastControl == null ? Offset(x, y) : Offset(2 * x - lastControl.dx, 2 * y - lastControl.dy);
        final x2 = number() + dx, y2 = number() + dy;
        x = number() + dx;
        y = number() + dy;
        path.cubicTo(reflected.dx, reflected.dy, x2, y2, x, y);
        control = Offset(x2, y2);
      case 'A':
        final rx = number(), ry = number(), rotation = number();
        final large = flag() == 1, sweep = flag() == 1;
        x = number() + dx;
        y = number() + dy;
        path.arcToPoint(Offset(x, y),
            radius: Radius.elliptical(rx, ry), rotation: rotation * math.pi / 180, largeArc: large, clockwise: sweep);
      case 'Z':
        path.close();
        x = startX;
        y = startY;
      case final other:
        throw FormatException('Path command $other is not drawn', d);
    }
    lastControl = control;
  }
  return path;
}
