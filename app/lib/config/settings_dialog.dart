import 'package:flutter/material.dart';
import 'canoa_config.dart';
import '../ledger/session_ledger.dart';
import '../theme/look.dart';

class SettingsDialog extends StatefulWidget {
  const SettingsDialog({super.key, required this.config, this.onQuit, this.ledger});
  final CanoaConfig config;

  /// The session list, whose kept text Settings can clear.
  final SessionLedger? ledger;

  /// Ends the app and what it started, on a desktop: the way out where there is no tray menu.
  final Future<void> Function()? onQuit;
  @override
  State<SettingsDialog> createState() => SettingsDialogState();
}
class SettingsDialogState extends State<SettingsDialog> {
  late CanoaConfig config = widget.config;

  LedgerSettings _sessions({int? keep, bool? keepText}) => LedgerSettings(
    keep: keep ?? config.sessions.keep,
    keepText: keepText ?? config.sessions.keepText,
    textMessages: config.sessions.textMessages,
    harnesses: config.sessions.harnesses,
  );

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Canoa settings'),
    content: SizedBox(width: 400, child: SingleChildScrollView(child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        ListTile(
          title: const Text('Look'),
          trailing: DropdownButton<LookId>(
            value: config.look,
            items: [for (final look in LookId.values) DropdownMenuItem(value: look, child: Text(look.label))],
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(look: value));
            },
          ),
        ),
        ListTile(
          title: const Text('Brightness'),
          trailing: DropdownButton<String>(
            value: config.brightness,
            items: const [
              DropdownMenuItem(value: 'dark', child: Text('Dark')),
              DropdownMenuItem(value: 'light', child: Text('Light')),
              DropdownMenuItem(value: 'system', child: Text('Follow the system')),
            ],
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(brightness: value));
            },
          ),
        ),
        SwitchListTile(
          title: const Text('Start hidden in tray'),
          value: config.startHidden,
          onChanged: (v) => setState(() => config = config.copyWith(startHidden: v)),
        ),
        SwitchListTile(
          title: const Text('Close window to tray'),
          value: config.closeToTray,
          onChanged: (v) => setState(() => config = config.copyWith(closeToTray: v)),
        ),
        SwitchListTile(
          title: const Text('Minimize window to tray'),
          value: config.minimizeToTray,
          onChanged: (v) => setState(() => config = config.copyWith(minimizeToTray: v)),
        ),
        SwitchListTile(
          title: const Text('Prefer WebTransport (WSS fallback)'),
          value: config.preferWebTransport,
          onChanged: (v) => setState(() =>
              config = config.copyWith(preferWebTransport: v)),
        ),
        ListTile(
          title: const Text('Network timeout'),
          subtitle: Text('${config.connectTimeout} seconds'),
          trailing: DropdownButton<int>(
            value: config.connectTimeout,
            items: [5,10,15,30,60].map((n) => DropdownMenuItem(
              value: n, child: Text('$n s'))).toList(),
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(connectTimeout: value));
            },
          ),
        ),
        ListTile(
          title: const Text('Sessions remembered'),
          subtitle: const Text('The least recently used go first'),
          trailing: DropdownButton<int>(
            value: config.sessions.keep,
            items: [
              for (final n in {0, 50, 200, 1000, config.sessions.keep}.toList()..sort())
                DropdownMenuItem(value: n, child: Text(n == 0 ? 'None' : '$n')),
            ],
            onChanged: (value) {
              if (value != null) setState(() => config = config.copyWith(sessions: _sessions(keep: value)));
            },
          ),
        ),
        SwitchListTile(
          title: const Text('Keep what was said with each session'),
          subtitle: const Text('Off: only the session\'s name and place are kept. Turning it off clears what was kept.'),
          value: config.sessions.keepText,
          onChanged: (v) => setState(() => config = config.copyWith(sessions: _sessions(keepText: v))),
        ),
        if (widget.ledger != null)
          ListenableBuilder(
            listenable: widget.ledger!,
            builder: (context, _) => ListTile(
              title: const Text('What was said, kept with sessions'),
              subtitle: Text(widget.ledger!.holdsText ? 'Some is kept' : 'None is kept'),
              trailing: TextButton(
                onPressed: widget.ledger!.holdsText ? widget.ledger!.clearText : null,
                child: const Text('Clear what was said'),
              ),
            ),
          ),
      ],
    ))),
    actions: [
      if (widget.onQuit != null)
        TextButton(onPressed: () => widget.onQuit!(), child: const Text('Quit Canoa')),
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.pop(context, config), child: const Text('Save')),
    ],
  );
}
