import 'package:flutter/material.dart';
import 'canoa_config.dart';
import '../ledger/session_ledger.dart';
import '../theme/look.dart';

class SettingsDialog extends StatefulWidget {
  const SettingsDialog({super.key, required this.config, this.onQuit, this.ledger, this.onProfiles, this.agents = const [],
      this.onAllowNotifications});
  final CanoaConfig config;

  /// The agents this app offers, by label, each of which full access can be turned on for.
  final List<String> agents;

  /// Asks the browser for leave to notify, where it has to be asked; true when it was given.
  final Future<bool> Function()? onAllowNotifications;

  /// The session list, whose kept text Settings can clear.
  final SessionLedger? ledger;

  /// Opens the effective profile of each agent.
  final VoidCallback? onProfiles;

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

  Future<bool> _confirmFullAccess(String agent) async =>
      await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text('Full access for $agent?'),
          content: Text('Each request $agent makes that offers to allow once will be allowed without asking you, in every '
              'conversation with it, until you turn this off here or on the conversation\'s banner.'),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
            FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Turn on full access')),
          ],
        ),
      ) ??
      false;

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
        ExpansionTile(
          key: const Key('settings-notifications'),
          title: const Text('Notifications'),
          subtitle: const Text('When a conversation you are not looking at needs you'),
          children: [
            for (final (label, on, set) in <(String, bool, NotificationSettings Function(bool))>[
              ('It asks leave to use a tool', config.notifications.permission,
                  (v) => config.notifications.copyWith(permission: v)),
              ('It has a question for you', config.notifications.question, (v) => config.notifications.copyWith(question: v)),
              ('It finished its turn', config.notifications.finished, (v) => config.notifications.copyWith(finished: v)),
            ])
              SwitchListTile(
                title: Text(label),
                value: on,
                onChanged: (v) => setState(() => config = config.copyWith(notifications: set(v))),
              ),
            if (widget.onAllowNotifications != null)
              ListTile(
                title: const Text('This browser'),
                subtitle: const Text('A browser shows them only once you allow it'),
                trailing: TextButton(
                  onPressed: () async {
                    final allowed = await widget.onAllowNotifications!();
                    if (!context.mounted) return;
                    ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(
                      content: Text(allowed ? 'This browser shows notifications from Canoa.' : 'This browser did not allow them.'),
                    ));
                  },
                  child: const Text('Allow notifications'),
                ),
              ),
          ],
        ),
        if (widget.agents.isNotEmpty || config.fullAccess.isNotEmpty)
          ExpansionTile(
            key: const Key('settings-full-access'),
            title: const Text('Full access'),
            subtitle: Text(config.fullAccess.isEmpty ? 'Off for every agent' : 'On for ${config.fullAccess.join(', ')}'),
            children: [
              const ListTile(
                dense: true,
                subtitle: Text('An agent with full access has each request that offers to allow once allowed without asking you. '
                    'A request offering no way to allow once is still asked. It changes nothing the browser lets pages or agents do.'),
              ),
              for (final agent in {...widget.agents, ...config.fullAccess})
                SwitchListTile(
                  title: Text(agent),
                  value: config.fullAccess.contains(agent),
                  onChanged: (on) async {
                    if (on && !await _confirmFullAccess(agent)) return;
                    final names = {...config.fullAccess};
                    on ? names.add(agent) : names.remove(agent);
                    setState(() => config = config.copyWith(fullAccess: names));
                  },
                ),
            ],
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
        if (widget.onProfiles != null)
          ListTile(
            title: const Text('Profiles'),
            subtitle: const Text('What each agent starts with, and the layer each value comes from'),
            trailing: TextButton(
              key: const Key('settings-profiles'),
              onPressed: widget.onProfiles,
              child: const Text('Show…'),
            ),
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
