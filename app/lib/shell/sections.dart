import 'package:flutter/material.dart';

/// One area of the app: what the selector shows for it, the index beside the main view, and the main view.
///
/// The app builds the list from its own parts; nothing outside it registers a section yet, so the shape stays free to change.
class AppSection {
  const AppSection({
    required this.id,
    required this.label,
    required this.icon,
    required this.main,
    this.index,
    this.tooltip,
    this.atFoot = false,
    this.ownsLayoutControls = false,
  });

  final String id;

  /// The name the selector's tooltip and screen reader say.
  final String label;
  final IconData icon;

  /// A longer tooltip, when the label alone does not say enough.
  final String? tooltip;

  /// The section's own list or tree, beside the main view; none for a section that has nothing to choose from.
  final Widget? index;

  /// What the section shows.
  final Widget main;

  /// Whether the main view draws the layout controls itself, in its own header. A section that does not gets the index toggle
  /// from the shell, at the main view's top right.
  final bool ownsLayoutControls;

  /// Whether the selector puts it at its foot, with the account.
  final bool atFoot;
}
