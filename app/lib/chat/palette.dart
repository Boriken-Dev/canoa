import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// The modifier of every shortcut: Command on a Mac, Control elsewhere.
bool get _onMac => defaultTargetPlatform == TargetPlatform.macOS;

/// A shortcut with the modifier this platform uses.
SingleActivator chord(LogicalKeyboardKey key, {bool shift = false}) =>
    SingleActivator(key, control: !_onMac, meta: _onMac, shift: shift);

/// One line of the help sheet: the keys and what they do.
typedef ShortcutLine = ({String keys, String what});

/// Every shortcut the app has, as the help sheet lists them.
List<ShortcutLine> get shortcutHelp {
  final mod = _onMac ? '⌘' : 'Ctrl';
  return [
    (keys: '$mod+K', what: 'Open the command palette'),
    (keys: '$mod+N', what: 'Start a new session'),
    (keys: '$mod+1 … $mod+9', what: 'Go to that session in the list'),
    (keys: '$mod+Tab, $mod+Shift+Tab', what: 'Go to the next or previous session'),
    (keys: '$mod+L', what: 'Write a message'),
    (keys: '↑ in an empty message', what: 'Bring back the last message sent here'),
    (keys: '$mod+.', what: 'Stop the turn'),
    (keys: '$mod+Shift+A', what: 'Allow once what the agent asks'),
    (keys: '$mod+Shift+R', what: 'Refuse once what the agent asks'),
    (keys: '$mod+Shift+D', what: 'Show the changes'),
    (keys: '$mod+/', what: 'Show these shortcuts'),
  ];
}

/// The keyboard shortcuts, for the person to read.
Future<void> showShortcutHelp(BuildContext context) => showDialog<void>(
  context: context,
  builder: (context) => AlertDialog(
    title: const Text('Keyboard shortcuts'),
    content: SizedBox(
      width: 460,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          for (final line in shortcutHelp)
            ListTile(
              dense: true,
              title: Text(line.what),
              trailing: Text(line.keys, style: const TextStyle(fontWeight: FontWeight.w600)),
            ),
        ]),
      ),
    ),
    actions: [TextButton(onPressed: () => Navigator.pop(context), child: const Text('Close'))],
  ),
);

/// Something the palette can do.
final class PaletteItem {
  const PaletteItem(this.label, this.run, {this.detail, this.icon = Icons.chevron_right});
  final String label;
  final String? detail;
  final IconData icon;
  final VoidCallback run;
}

/// A searchable list of what the app can do, sessions to go to and the agent's own commands. Enter runs the first match.
class CommandPalette extends StatefulWidget {
  const CommandPalette({super.key, required this.items});

  final List<PaletteItem> items;

  @override
  State<CommandPalette> createState() => _CommandPaletteState();
}

class _CommandPaletteState extends State<CommandPalette> {
  String _typed = '';

  List<PaletteItem> get _matching {
    final words = _typed.toLowerCase().split(' ').where((word) => word.isNotEmpty);
    return [
      for (final item in widget.items)
        if (words.every((word) => '${item.label} ${item.detail ?? ''}'.toLowerCase().contains(word))) item,
    ];
  }

  void _run(PaletteItem item) {
    Navigator.pop(context);
    item.run();
  }

  @override
  Widget build(BuildContext context) {
    final matching = _matching;
    return Dialog(
      alignment: Alignment.topCenter,
      insetPadding: const EdgeInsets.fromLTRB(16, 64, 16, 16),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560, maxHeight: 480),
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          Padding(
            padding: const EdgeInsets.all(12),
            child: TextField(
              key: const Key('palette-search'),
              autofocus: true,
              decoration: const InputDecoration(hintText: 'Type to find an action, a session or a command', prefixIcon: Icon(Icons.search)),
              textInputAction: TextInputAction.done,
              onChanged: (typed) => setState(() => _typed = typed),
              onSubmitted: (_) {
                if (matching.isNotEmpty) _run(matching.first);
              },
            ),
          ),
          Flexible(
            child: ListView(shrinkWrap: true, children: [
              for (final item in matching)
                ListTile(
                  dense: true,
                  leading: Icon(item.icon, size: 18),
                  title: Text(item.label, maxLines: 1, overflow: TextOverflow.ellipsis),
                  subtitle: item.detail == null ? null : Text(item.detail!, maxLines: 1, overflow: TextOverflow.ellipsis),
                  onTap: () => _run(item),
                ),
              if (matching.isEmpty) const ListTile(title: Text('Nothing matches')),
            ]),
          ),
        ]),
      ),
    );
  }
}
