import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../selectable.dart';

/// The modifier of every shortcut: Command on a Mac, Control elsewhere.
bool get _onMac => defaultTargetPlatform == TargetPlatform.macOS;

/// A shortcut with the modifier this platform uses.
SingleActivator chord(LogicalKeyboardKey key, {bool shift = false, bool alt = false}) =>
    SingleActivator(key, control: !_onMac, meta: _onMac, shift: shift, alt: alt);

/// The keys that start and end a dictation, as the tooltip and the help sheet name them.
String get dictationKeys => '${_onMac ? "⌘" : "Ctrl"}+Shift+Space';

/// One line of the help sheet: the keys and what they do.
typedef ShortcutLine = ({String keys, String what});

/// One entry in the single shortcut table feeding both window bindings and help / Settings.
final class ShortcutSpec {
  const ShortcutSpec({
    required this.id,
    required this.what,
    required this._defaultKeys,
    this.defaultActivator,
  });

  final String id;
  final String what;
  final String Function(String mod) _defaultKeys;
  final SingleActivator Function()? defaultActivator;

  bool get rebindable => defaultActivator != null;
  String defaultLabel() => _defaultKeys(_onMac ? '⌘' : 'Ctrl');

  String keysWith(Map<String, String> overrides) {
    final custom = overrides[id];
    return custom != null && custom.trim().isNotEmpty ? custom.trim() : defaultLabel();
  }

  SingleActivator? activatorWith(Map<String, String> overrides) {
    final builder = defaultActivator;
    if (builder == null) return null;
    final custom = overrides[id];
    if (custom != null && custom.trim().isNotEmpty) {
      return parseShortcutActivator(custom) ?? builder();
    }
    return builder();
  }
}

/// Parses a human-readable key chord (`Ctrl+Shift+K`, `Alt+B`, `Esc`) into a [SingleActivator], or null when invalid.
SingleActivator? parseShortcutActivator(String spec) {
  final parts = spec.split('+').map((s) => s.trim()).where((s) => s.isNotEmpty).toList();
  if (parts.isEmpty) return null;
  var control = false, meta = false, shift = false, alt = false;
  for (var i = 0; i < parts.length - 1; i++) {
    switch (parts[i].toLowerCase()) {
      case 'ctrl' || 'control':
        control = true;
      case 'cmd' || 'command' || 'meta' || '⌘':
        meta = true;
      case 'mod':
        if (_onMac) {
          meta = true;
        } else {
          control = true;
        }
      case 'shift' || '⇧':
        shift = true;
      case 'alt' || 'option':
        alt = true;
      default:
        return null;
    }
  }
  final key = _parseKey(parts.last.toLowerCase());
  if (key == null) return null;
  return SingleActivator(key, control: control, meta: meta, shift: shift, alt: alt);
}

LogicalKeyboardKey? _parseKey(String name) => switch (name) {
  'esc' || 'escape' => LogicalKeyboardKey.escape,
  'tab' => LogicalKeyboardKey.tab,
  'space' => LogicalKeyboardKey.space,
  ',' || 'comma' => LogicalKeyboardKey.comma,
  '.' || 'period' => LogicalKeyboardKey.period,
  '/' || 'slash' => LogicalKeyboardKey.slash,
  '`' || 'backquote' => LogicalKeyboardKey.backquote,
  _ when name.length == 1 && name.codeUnitAt(0) >= 97 && name.codeUnitAt(0) <= 122 =>
    LogicalKeyboardKey(LogicalKeyboardKey.keyA.keyId + name.codeUnitAt(0) - 97),
  _ when name.length == 1 && name.codeUnitAt(0) >= 48 && name.codeUnitAt(0) <= 57 =>
    LogicalKeyboardKey(LogicalKeyboardKey.digit0.keyId + name.codeUnitAt(0) - 48),
  _ => null,
};

/// The one canonical shortcut table: bindings in `app_shortcuts.dart`, [showShortcutHelp], and Settings > Shortcuts all read this.
final List<ShortcutSpec> appShortcutTable = [
  ShortcutSpec(id: 'palette', what: 'Open the command palette', defaultKeys: (m) => '$m+K', defaultActivator: () => chord(LogicalKeyboardKey.keyK)),
  ShortcutSpec(id: 'find', what: 'Find in the conversation', defaultKeys: (m) => '$m+F', defaultActivator: () => chord(LogicalKeyboardKey.keyF)),
  ShortcutSpec(id: 'new_session', what: 'Start a new session', defaultKeys: (m) => '$m+N', defaultActivator: () => chord(LogicalKeyboardKey.keyN)),
  ShortcutSpec(id: 'close_session', what: 'Close the active conversation', defaultKeys: (m) => '$m+W', defaultActivator: () => chord(LogicalKeyboardKey.keyW)),
  ShortcutSpec(id: 'toggle_index', what: 'Hide or show the section index', defaultKeys: (m) => '$m+B', defaultActivator: () => chord(LogicalKeyboardKey.keyB)),
  ShortcutSpec(id: 'toggle_right', what: 'Hide or show the right panel', defaultKeys: (m) => '$m+Alt+B', defaultActivator: () => chord(LogicalKeyboardKey.keyB, alt: true)),
  ShortcutSpec(id: 'toggle_bottom', what: 'Hide or show the bottom panel', defaultKeys: (m) => '$m+`', defaultActivator: () => chord(LogicalKeyboardKey.backquote)),
  ShortcutSpec(id: 'settings', what: 'Open Settings', defaultKeys: (m) => '$m+,', defaultActivator: () => chord(LogicalKeyboardKey.comma)),
  ShortcutSpec(id: 'dictation', what: 'Dictate into the message, and stop', defaultKeys: (_) => dictationKeys, defaultActivator: () => chord(LogicalKeyboardKey.space, shift: true)),
  ShortcutSpec(id: 'paste_file', what: 'Paste: an image or files go with it, long text as a file', defaultKeys: (m) => '$m+V in a message'),
  ShortcutSpec(id: 'select_all', what: 'Select all of the conversation, or of the message being written', defaultKeys: (m) => '$m+A'),
  ShortcutSpec(id: 'copy', what: 'Copy what is selected', defaultKeys: (m) => '$m+C'),
  ShortcutSpec(id: 'session_digit', what: 'Go to that session in the list', defaultKeys: (m) => '$m+1 … $m+9'),
  ShortcutSpec(id: 'next_session', what: 'Go to the next or previous session', defaultKeys: (m) => '$m+Tab, $m+Shift+Tab'),
  ShortcutSpec(id: 'focus_composer', what: 'Write a message', defaultKeys: (m) => '$m+L', defaultActivator: () => chord(LogicalKeyboardKey.keyL)),
  ShortcutSpec(id: 'recall_prompt', what: 'Bring back the last message sent here', defaultKeys: (_) => '↑ in an empty message'),
  ShortcutSpec(id: 'stop_turn', what: 'Stop the turn', defaultKeys: (m) => '$m+.', defaultActivator: () => chord(LogicalKeyboardKey.period)),
  ShortcutSpec(id: 'stop_turn_esc', what: 'Stop the working turn', defaultKeys: (_) => 'Esc', defaultActivator: () => const SingleActivator(LogicalKeyboardKey.escape)),
  ShortcutSpec(id: 'allow_once', what: 'Allow once what the agent asks', defaultKeys: (m) => '$m+Shift+A', defaultActivator: () => chord(LogicalKeyboardKey.keyA, shift: true)),
  ShortcutSpec(id: 'reject_once', what: 'Refuse once what the agent asks', defaultKeys: (m) => '$m+Shift+R', defaultActivator: () => chord(LogicalKeyboardKey.keyR, shift: true)),
  ShortcutSpec(id: 'show_changes', what: 'Show the changes', defaultKeys: (m) => '$m+Shift+D', defaultActivator: () => chord(LogicalKeyboardKey.keyD, shift: true)),
  ShortcutSpec(id: 'shortcut_help', what: 'Show these shortcuts', defaultKeys: (m) => '$m+/', defaultActivator: () => chord(LogicalKeyboardKey.slash)),
];

/// Every shortcut the app has, with any [overrides] from `[shortcuts]` in canoa.cfg applied.
List<ShortcutLine> shortcutHelpFor([Map<String, String> overrides = const {}]) => [
  for (final spec in appShortcutTable) (keys: spec.keysWith(overrides), what: spec.what),
];

/// Default shortcut help list, matching [appShortcutTable].
List<ShortcutLine> get shortcutHelp => shortcutHelpFor();

/// The keyboard shortcuts, for the person to read.
Future<void> showShortcutHelp(BuildContext context, {Map<String, String> overrides = const {}}) => showSelectableDialog<void>(
  context: context,
  builder: (context) => AlertDialog(
    title: const Text('Keyboard shortcuts'),
    content: SizedBox(
      width: 460,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          for (final line in shortcutHelpFor(overrides))
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
