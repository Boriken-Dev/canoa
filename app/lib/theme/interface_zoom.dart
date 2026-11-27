import 'package:material_ui/material_ui.dart';

/// The smallest and largest factor `[appearance] zoom` may name: below the first nothing is legible, and a factor of 0 or less cannot
/// be laid out.
const double minZoom = 0.5, maxZoom = 3;

/// The factors Ctrl+= and Ctrl+- move through, in order.
const List<double> zoomSteps = [0.5, 0.67, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3];

/// The factor one step from [factor] toward larger ([direction] above 0) or smaller (below 0); [factor] itself at an end. A factor
/// between two steps moves to the next one in that direction.
double steppedZoom(double factor, int direction) {
  if (direction > 0) return zoomSteps.firstWhere((step) => step > factor + 0.001, orElse: () => factor);
  return zoomSteps.lastWhere((step) => step < factor - 0.001, orElse: () => factor);
}

/// Draws [child] [factor] times as large, whole: text, icons, spacing, and what a pointer lands on. The child is laid out in a window
/// 1 / [factor] the size of this one, so it reflows as it would in a smaller or larger window.
class InterfaceZoom extends StatelessWidget {
  const InterfaceZoom({super.key, required this.factor, required this.child});

  final double factor;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (factor == 1) return child;
    final media = MediaQuery.of(context);
    return LayoutBuilder(
      builder: (context, constraints) {
        final outer = constraints.biggest;
        final inner = Size(outer.width / factor, outer.height / factor);
        return Transform.scale(
          scale: factor,
          alignment: Alignment.topLeft,
          child: OverflowBox(
            alignment: Alignment.topLeft,
            minWidth: inner.width,
            maxWidth: inner.width,
            minHeight: inner.height,
            maxHeight: inner.height,
            child: MediaQuery(
              data: media.copyWith(
                size: inner,
                padding: media.padding / factor,
                viewPadding: media.viewPadding / factor,
                viewInsets: media.viewInsets / factor,
              ),
              child: child,
            ),
          ),
        );
      },
    );
  }
}
