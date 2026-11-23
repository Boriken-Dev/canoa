import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../theme/look.dart';
import 'desktop_shell.dart';

/// Keeps the tray's words and the colour of its icon in step with the look in force, for a [shell] that has a tray to restyle.
///
/// It draws nothing of its own: it passes [child] through, and tells the shell after each frame in which the text or the accent
/// changed.
class TrayFollower extends StatefulWidget {
  const TrayFollower({super.key, required this.shell, required this.child});

  final DesktopShell? shell;
  final Widget child;

  @override
  State<TrayFollower> createState() => _TrayFollowerState();
}

class _TrayFollowerState extends State<TrayFollower> {
  @override
  Widget build(BuildContext context) {
    final shell = widget.shell;
    if (shell is TrayAppearance) {
      final text = AppText.of(context);
      final accent = context.look.accent;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) unawaited((shell as TrayAppearance).applyAppearance(text: text, accent: accent));
      });
    }
    return widget.child;
  }
}
