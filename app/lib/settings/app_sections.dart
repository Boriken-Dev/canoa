import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity, ChatLook;
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';

import '../chat/palette.dart' show shortcutHelp;
import '../config/canoa_config.dart';
import '../ledger/session_ledger.dart';
import '../theme/look.dart';
import '../workspace/workspace_template.dart';
import 'settings_controller.dart';
import 'settings_row.dart';

/// Tray and window, the network, and what the session list keeps.
class GeneralSection extends StatefulWidget {
  const GeneralSection({super.key, required this.controller, this.ledger, this.workspacePreview});

  final SettingsController controller;
  final SessionLedger? ledger;

  /// The folder a template makes for a session, as the app would make it now; null where this build has no folders.
  final String? Function(String template)? workspacePreview;

  @override
  State<GeneralSection> createState() => _GeneralSectionState();
}

class _GeneralSectionState extends State<GeneralSection> {
  late final _workspace = TextEditingController(text: widget.controller.config.sessions.workspace);
  String? _workspaceProblem;

  @override
  void dispose() {
    _workspace.dispose();
    super.dispose();
  }

  LedgerSettings _sessions(CanoaConfig config, {int? keep, bool? keepText, String? workspace}) => LedgerSettings(
    keep: keep ?? config.sessions.keep,
    keepText: keepText ?? config.sessions.keepText,
    textMessages: config.sessions.textMessages,
    workspace: workspace ?? config.sessions.workspace,
    harnesses: config.sessions.harnesses,
  );

  void _setWorkspace(String template, {bool text = false}) {
    if (text) _workspace.text = template;
    setState(() => _workspaceProblem = workspaceProblem(template));
    if (_workspaceProblem == null) widget.controller.change((c) => c.copyWith(sessions: _sessions(c, workspace: template)));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final c = widget.controller;
      final config = c.config;
      final problem = _workspaceProblem;
      final preview = problem == null ? widget.workspacePreview?.call(_workspace.text) : null;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (!kIsWeb)
          SettingsGroup(title: 'Window', children: [
            SwitchRow(
              title: 'Start hidden in the tray',
              description: 'Canoa starts with no window; open it from the tray icon',
              machine: true,
              value: config.startHidden,
              onChanged: (v) => c.change((x) => x.copyWith(startHidden: v)),
            ),
            SwitchRow(
              title: 'Close the window to the tray',
              description: 'Closing keeps Canoa and its sessions running',
              machine: true,
              value: config.closeToTray,
              onChanged: (v) => c.change((x) => x.copyWith(closeToTray: v)),
            ),
            SwitchRow(
              title: 'Minimize the window to the tray',
              description: 'Minimizing hides the window instead of leaving it on the taskbar',
              machine: true,
              value: config.minimizeToTray,
              onChanged: (v) => c.change((x) => x.copyWith(minimizeToTray: v)),
            ),
          ]),
        SettingsGroup(title: 'Network', children: [
          SwitchRow(
            title: 'Prefer WebTransport',
            description: 'Falls back to WSS when WebTransport does not connect',
            value: config.preferWebTransport,
            onChanged: (v) => c.change((x) => x.copyWith(preferWebTransport: v)),
          ),
          ChoiceRow<int>(
            title: 'Network timeout',
            description: 'How long a connection may take before Canoa gives up',
            child: true,
            fieldKey: const Key('settings-timeout'),
            value: config.connectTimeout,
            choices: [for (final n in {5, 10, 15, 30, 60, config.connectTimeout}.toList()..sort()) (n, '$n s')],
            onChanged: (v) => c.change((x) => x.copyWith(connectTimeout: v)),
          ),
        ]),
        SettingsGroup(title: 'Sessions', children: [
          ChoiceRow<int>(
            title: 'Sessions remembered',
            description: 'The least recently used go first',
            fieldKey: const Key('settings-keep'),
            value: config.sessions.keep,
            choices: [for (final n in {0, 50, 200, 1000, config.sessions.keep}.toList()..sort()) (n, n == 0 ? 'None' : '$n')],
            onChanged: (v) => c.change((x) => x.copyWith(sessions: _sessions(x, keep: v))),
          ),
          SwitchRow(
            key: const Key('settings-keep-text'),
            title: 'Keep what was said with each session',
            description: 'Off: only the session\'s name and place are kept. Turning it off clears what was kept',
            child: true,
            machine: true,
            enabled: config.sessions.keep > 0,
            value: config.sessions.keepText,
            onChanged: (v) => c.change((x) => x.copyWith(sessions: _sessions(x, keepText: v))),
          ),
          if (widget.ledger case final ledger?)
            ListenableBuilder(
              listenable: ledger,
              builder: (context, _) => SettingsRow(
                title: 'What was said, kept with sessions',
                description: ledger.holdsText ? 'Some is kept' : 'None is kept',
                child: true,
                enabled: config.sessions.keep > 0,
                control: TextButton(
                  key: const Key('settings-clear-text'),
                  onPressed: ledger.holdsText ? ledger.clearText : null,
                  child: const Text('Clear what was said'),
                ),
              ),
            ),
          if (widget.workspacePreview != null)
            SettingsRow(
              title: 'Folder of a session with none chosen',
              description: preview == null ? 'A template; the folder it makes is shown under the field' : 'Now: $preview',
              machine: true,
              below: TextField(
                key: const Key('settings-workspace'),
                controller: _workspace,
                decoration: InputDecoration(
                  isDense: true,
                  errorText: problem,
                  errorMaxLines: 3,
                  border: const OutlineInputBorder(),
                  suffixIcon: _workspace.text == defaultWorkspace
                      ? null
                      : IconButton(
                          key: const Key('settings-workspace-reset'),
                          tooltip: 'Use the default',
                          icon: const Icon(Icons.restart_alt),
                          onPressed: () => _setWorkspace(defaultWorkspace, text: true),
                        ),
                ),
                onChanged: _setWorkspace,
              ),
            ),
        ]),
      ]);
    },
  );
}

/// Look, brightness and the detail of the agent's work, with a preview of the choice.
class AppearanceSection extends StatelessWidget {
  const AppearanceSection({super.key, required this.controller});

  final SettingsController controller;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final config = controller.config;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        SettingsGroup(title: 'Theme', children: [
          ChoiceRow<LookId>(
            title: 'Look',
            description: 'Bubbles on a warm page, or a terminal\'s lines',
            fieldKey: const Key('settings-look'),
            value: config.look,
            choices: [for (final look in LookId.values) (look, look.label)],
            onChanged: (v) => controller.change((c) => c.copyWith(look: v)),
          ),
          ChoiceRow<String>(
            title: 'Brightness',
            description: 'Follow the system, or stay dark or light',
            fieldKey: const Key('settings-brightness'),
            value: config.brightness,
            choices: const [('dark', 'Dark'), ('light', 'Light'), ('system', 'Follow the system')],
            onChanged: (v) => controller.change((c) => c.copyWith(brightness: v)),
          ),
          ChoiceRow<ChatDensity>(
            title: 'Detail of the agent\'s work',
            description: 'Normal folds a finished turn into one line; Verbose shows every step',
            fieldKey: const Key('settings-density'),
            value: config.density,
            choices: [for (final density in ChatDensity.values) (density, density.label)],
            onChanged: (v) => controller.change((c) => c.copyWith(density: v)),
          ),
        ]),
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 0, 4, 8),
          child: Text('Preview', style: Theme.of(context).textTheme.titleSmall),
        ),
        AppearancePreview(look: config.look, brightness: config.brightness, density: config.density),
      ]);
    },
  );
}

/// A short conversation with a diff, drawn in the look and brightness chosen, whichever the app shows now.
class AppearancePreview extends StatelessWidget {
  const AppearancePreview({super.key, required this.look, required this.brightness, required this.density});

  final LookId look;
  final String brightness;
  final ChatDensity density;

  @override
  Widget build(BuildContext context) {
    final bright = switch (brightness) {
      'light' => Brightness.light,
      'system' => MediaQuery.platformBrightnessOf(context),
      _ => Brightness.dark,
    };
    final theme = themeFor(look, bright, density: density);
    final chat = theme.extension<ChatLook>()!;
    final mono = TextStyle(fontFamily: chat.codeFont, fontSize: 13, color: chat.text);
    Widget line(String sign, String text, Color? background) => Container(
      color: background,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 1),
      child: Text('$sign $text', style: mono),
    );
    return Theme(
      data: theme,
      child: Container(
        key: const Key('appearance-preview'),
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(color: chat.bg, borderRadius: BorderRadius.circular(12), border: Border.all(color: chat.border)),
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Align(
            alignment: Alignment.centerRight,
            child: Container(
              padding: EdgeInsets.all(chat.padding),
              decoration: BoxDecoration(color: chat.user, borderRadius: BorderRadius.circular(chat.radius)),
              child: Text('Make the sum test pass', style: TextStyle(color: chat.text, fontSize: chat.bodySize)),
            ),
          ),
          SizedBox(height: chat.gap / 2),
          Text('The sum was subtracting. I changed it to add:', style: TextStyle(color: chat.text, fontSize: chat.bodySize)),
          const SizedBox(height: 8),
          ClipRRect(
            borderRadius: BorderRadius.circular(chat.radius / 2),
            child: Container(
              color: chat.code,
              padding: const EdgeInsets.symmetric(vertical: 6),
              child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                line('-', 'return a - b;', chat.removed),
                line('+', 'return a + b;', chat.added),
              ]),
            ),
          ),
          const SizedBox(height: 8),
          Text('2 tests passed', style: TextStyle(color: chat.muted, fontSize: 13)),
        ]),
      ),
    );
  }
}

/// One row per kind of news from a session, each a switch.
class NotificationsSection extends StatelessWidget {
  const NotificationsSection({super.key, required this.controller, this.onAllowNotifications});

  final SettingsController controller;

  /// Asks the browser for leave to notify, where it has to be asked; true when it was given.
  final Future<bool> Function()? onAllowNotifications;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final n = controller.config.notifications;
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        SettingsGroup(title: 'When a conversation you are not looking at', children: [
          SwitchRow(
            title: 'It asks leave to use a tool',
            description: 'A permission request is waiting for your answer',
            value: n.permission,
            onChanged: (v) => controller.change((c) => c.copyWith(notifications: c.notifications.copyWith(permission: v))),
          ),
          SwitchRow(
            title: 'It has a question for you',
            description: 'The agent stopped to ask, and waits',
            value: n.question,
            onChanged: (v) => controller.change((c) => c.copyWith(notifications: c.notifications.copyWith(question: v))),
          ),
          SwitchRow(
            title: 'It finished its turn',
            description: 'The agent is done and has said what it did',
            value: n.finished,
            onChanged: (v) => controller.change((c) => c.copyWith(notifications: c.notifications.copyWith(finished: v))),
          ),
        ]),
        if (onAllowNotifications != null)
          SettingsGroup(title: 'This browser', children: [
            SettingsRow(
              title: 'Allow notifications',
              description: 'A browser shows them only once you allow it',
              machine: true,
              control: TextButton(
                key: const Key('settings-allow-notifications'),
                onPressed: () async {
                  final allowed = await onAllowNotifications!();
                  if (!context.mounted) return;
                  ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(
                    content: Text(allowed ? 'This browser shows notifications from Canoa.' : 'This browser did not allow them.'),
                  ));
                },
                child: const Text('Allow'),
              ),
            ),
          ]),
      ]);
    },
  );
}

/// The keyboard shortcuts, for reading.
class ShortcutsSection extends StatelessWidget {
  const ShortcutsSection({super.key});

  @override
  Widget build(BuildContext context) => SettingsGroup(children: [
    for (final line in shortcutHelp)
      SettingsRow(
        title: line.what,
        control: Text(line.keys, style: const TextStyle(fontWeight: FontWeight.w600)),
      ),
  ]);
}
