/// How an overlay's knowledge reaches the agent: put in its context whole, or left out and searched through the knowledge tools
/// (`[overlays.NAME] kb`). Left alone, the overlay's own hint decides.
library;

import 'package:material_ui/material_ui.dart';

/// The three choices of one overlay: [choice] is `index`, `files` or null for the overlay's own hint, which [hinted] says is
/// to search.
class OverlayKnowledge extends StatelessWidget {
  const OverlayKnowledge({super.key, required this.overlay, required this.choice, required this.hinted, required this.onChanged,
      this.busy = false});

  final String overlay;
  final String? choice;

  /// Whether the overlay's manifest says its knowledge is meant to be searched (`[knowledge] serve = true`).
  final bool hinted;
  final ValueChanged<String?> onChanged;
  final bool busy;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final effective = choice ?? (hinted ? 'index' : 'files');
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      Text('Its rules and context', style: text.labelLarge),
      const SizedBox(height: 4),
      SegmentedButton<String?>(
        key: Key('overlay-kb-$overlay'),
        showSelectedIcon: false,
        emptySelectionAllowed: false,
        segments: [
          ButtonSegment(value: null, label: Text(hinted ? 'Default: searchable' : 'Default: whole files')),
          const ButtonSegment(value: 'files', label: Text('Whole files')),
          const ButtonSegment(value: 'index', label: Text('Searchable')),
        ],
        selected: {choice},
        onSelectionChanged: busy ? null : (chosen) => onChanged(chosen.first),
      ),
      const SizedBox(height: 4),
      Text(
        effective == 'index'
            ? 'Left out of the agent\'s context: it is given tools to search them and read the lines it needs. A session whose client '
                'does not ask for the tools keeps the whole files.'
            : 'Put in the agent\'s context whole, at the start of each session.',
        key: Key('overlay-kb-note-$overlay'),
        style: text.bodySmall,
      ),
    ]);
  }
}
