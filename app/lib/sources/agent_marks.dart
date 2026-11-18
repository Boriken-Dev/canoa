import 'package:flutter/painting.dart';

/// One vendor mark: its shapes in a square box [side] units wide. A shape's colour is the vendor's own where the mark is
/// coloured, and null where it is monochrome, drawn in the text colour.
final class AgentMark {
  const AgentMark(this.side, this.shapes);

  final double side;

  /// SVG path data and its fill.
  final List<(String, Color?)> shapes;
}

/// A mark from the SVG text an agent's description carries: the `viewBox`'s width as the box's side, and each `path` element's `d`
/// and `fill` (`#RRGGBB`), taken from the element or else from the root. A path with no colour, or `currentColor`, is drawn in the
/// text's. Null for text that holds no path or no square box.
AgentMark? parseMarkSvg(String svg) {
  final root = RegExp(r'<svg[^>]*>').firstMatch(svg)?[0];
  if (root == null) return null;
  final box = RegExp(r'viewBox="\s*0[ ,]+0[ ,]+(\d+(?:\.\d+)?)[ ,]+(\d+(?:\.\d+)?)\s*"').firstMatch(root);
  if (box == null) return null;
  final side = double.parse(box[1]!);
  if (side <= 0 || side != double.parse(box[2]!)) return null;
  Color? fillOf(String? element) {
    final fill = element == null ? null : RegExp(r'\sfill="#([0-9A-Fa-f]{6})"').firstMatch(element);
    return fill == null ? null : Color(0xFF000000 | int.parse(fill[1]!, radix: 16));
  }

  final shapes = <(String, Color?)>[];
  for (final element in RegExp(r'<path[^>]*>').allMatches(svg).map((each) => each[0]!)) {
    final d = RegExp(r'\sd="([^"]*)"').firstMatch(element)?[1];
    if (d == null || d.isEmpty) continue;
    shapes.add((d, fillOf(element) ?? fillOf(root)));
  }
  return shapes.isEmpty ? null : AgentMark(side, shapes);
}
