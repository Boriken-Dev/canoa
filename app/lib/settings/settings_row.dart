import 'package:flutter/material.dart';

/// The row every Settings section is made of: a title and a one-line description on the left, the control on the right.
///
/// A [child] row is a sub-option: it is indented under its parent and, with [enabled] false, drawn and disabled with it.
/// [machine] marks a value that is kept only on this computer.
class SettingsRow extends StatelessWidget {
  const SettingsRow({
    super.key,
    required this.title,
    this.description,
    this.control,
    this.child = false,
    this.enabled = true,
    this.machine = false,
    this.below,
  });

  final String title;
  final String? description;

  /// The control, at the right edge.
  final Widget? control;

  /// Indents the row under the one above it.
  final bool child;

  /// False greys the row and its control out; the control is wrapped so it cannot be used.
  final bool enabled;

  /// Whether the value is kept only on this computer.
  final bool machine;

  /// Full-width content under the title, such as a field or a list.
  final Widget? below;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final muted = scheme.onSurfaceVariant;
    final label = Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Wrap(crossAxisAlignment: WrapCrossAlignment.center, spacing: 8, children: [
        Text(title, style: text.bodyLarge),
        if (machine) const MachineBadge(),
      ]),
      if (description != null) Padding(padding: const EdgeInsets.only(top: 2), child: Text(description!, style: text.bodySmall?.copyWith(color: muted))),
    ]);
    final body = Padding(
      padding: EdgeInsets.fromLTRB(child ? 40 : 16, 12, 16, 12),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        LayoutBuilder(builder: (context, constraints) {
          // A phone has no room for both side by side: the control goes under the text.
          if (control == null) return label;
          if (constraints.maxWidth < 420) {
            return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              label,
              const SizedBox(height: 8),
              Align(alignment: Alignment.centerLeft, child: control),
            ]);
          }
          return Row(crossAxisAlignment: CrossAxisAlignment.center, children: [
            Expanded(child: label),
            const SizedBox(width: 16),
            control!,
          ]);
        }),
        if (below != null) Padding(padding: const EdgeInsets.only(top: 8), child: below),
      ]),
    );
    if (enabled) return body;
    return Opacity(opacity: 0.45, child: IgnorePointer(child: ExcludeFocus(child: body)));
  }
}

/// "This computer": a value the row says is kept here only.
class MachineBadge extends StatelessWidget {
  const MachineBadge({super.key});

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      key: const Key('machine-badge'),
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
      decoration: BoxDecoration(border: Border.all(color: scheme.outline), borderRadius: BorderRadius.circular(4)),
      child: Text('This computer', style: Theme.of(context).textTheme.labelSmall?.copyWith(color: scheme.onSurfaceVariant)),
    );
  }
}

/// A titled group of rows in a rounded card.
class SettingsGroup extends StatelessWidget {
  const SettingsGroup({super.key, this.title, this.trailing, required this.children});

  final String? title;
  final Widget? trailing;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.only(bottom: 20),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (title != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 0, 4, 8),
            child: Row(children: [
              Expanded(child: Text(title!, style: Theme.of(context).textTheme.titleSmall)),
              ?trailing,
            ]),
          ),
        DecoratedBox(
          decoration: BoxDecoration(
            border: Border.all(color: scheme.outlineVariant),
            borderRadius: BorderRadius.circular(12),
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(12),
            child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              for (var i = 0; i < children.length; i++) ...[
                if (i > 0) Divider(height: 1, color: scheme.outlineVariant),
                children[i],
              ],
            ]),
          ),
        ),
      ]),
    );
  }
}

/// A row whose control is a switch.
class SwitchRow extends StatelessWidget {
  const SwitchRow({
    super.key,
    required this.title,
    this.description,
    required this.value,
    required this.onChanged,
    this.child = false,
    this.enabled = true,
    this.machine = false,
  });

  final String title;
  final String? description;
  final bool value;
  final ValueChanged<bool> onChanged;
  final bool child, enabled, machine;

  @override
  Widget build(BuildContext context) => SettingsRow(
    title: title,
    description: description,
    child: child,
    enabled: enabled,
    machine: machine,
    control: Switch(value: value, onChanged: enabled ? onChanged : null),
  );
}

/// A row whose control is a dropdown.
class ChoiceRow<T> extends StatelessWidget {
  const ChoiceRow({
    super.key,
    required this.title,
    this.description,
    required this.value,
    required this.choices,
    required this.onChanged,
    this.child = false,
    this.enabled = true,
    this.machine = false,
    this.fieldKey,
  });

  final String title;
  final String? description;
  final T value;

  /// The values and what each is called.
  final List<(T, String)> choices;
  final ValueChanged<T> onChanged;
  final bool child, enabled, machine;

  /// The key of the dropdown itself.
  final Key? fieldKey;

  @override
  Widget build(BuildContext context) => SettingsRow(
    title: title,
    description: description,
    child: child,
    enabled: enabled,
    machine: machine,
    control: DropdownButton<T>(
      key: fieldKey,
      value: value,
      items: [for (final (v, label) in choices) DropdownMenuItem<T>(value: v, child: Text(label))],
      onChanged: enabled ? (picked) => picked == null ? null : onChanged(picked) : null,
    ),
  );
}

/// "Applying settings for [scope]": the choice, on a section whose values can be set per agent, of the one this page edits.
class ScopeBar extends StatelessWidget {
  const ScopeBar({super.key, this.fieldKey, required this.label, required this.value, required this.choices, required this.onChanged});

  /// The key of the dropdown.
  final Key? fieldKey;

  /// What is being chosen, such as "agent".
  final String label;
  final String? value;

  /// The scopes, each by the name shown and kept.
  final List<String> choices;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 16),
    child: Wrap(crossAxisAlignment: WrapCrossAlignment.center, children: [
      Text('Applying settings for $label ', style: Theme.of(context).textTheme.bodyMedium),
      const SizedBox(width: 8),
      DropdownButton<String>(
        key: fieldKey ?? const Key('scope-bar'),
        value: choices.contains(value) ? value : null,
        hint: const Text('None yet'),
        items: [for (final scope in choices) DropdownMenuItem(value: scope, child: Text(scope))],
        onChanged: (picked) => picked == null ? null : onChanged(picked),
      ),
    ]),
  );
}
