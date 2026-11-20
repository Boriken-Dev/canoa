import 'package:flutter/widgets.dart';

/// [child] drawn, or kept built and out of sight. A subtree kept in a [Stack] under [Offstage] still has the keyboard if it held it
/// when it was hidden, so it goes on taking what the person types; here a hidden subtree cannot have focus, and loses it when it is
/// hidden. [Shown.of] tells a widget inside whether it is in front.
class Shown extends StatelessWidget {
  const Shown({super.key, required this.shown, required this.child});

  final bool shown;
  final Widget child;

  /// Whether the widget at [context] is in front: no [Shown] above it hides it.
  static bool of(BuildContext context) => context.dependOnInheritedWidgetOfExactType<_ShownScope>()?.shown ?? true;

  @override
  Widget build(BuildContext context) => ExcludeFocus(
    excluding: !shown,
    child: _ShownScope(
      shown: shown && Shown.of(context),
      child: Offstage(offstage: !shown, child: child),
    ),
  );
}

class _ShownScope extends InheritedWidget {
  const _ShownScope({required this.shown, required super.child});

  final bool shown;

  @override
  bool updateShouldNotify(_ShownScope old) => shown != old.shown;
}
