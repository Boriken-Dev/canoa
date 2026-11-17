import 'package:flutter/material.dart';

import '../acp/delegate_targets.dart';
import '../selectable.dart';
import '../session_store.dart';

/// What this conversation's agent may hand tasks to: the agents, the one a task with none named goes to, how many run at once, and
/// whether the agent may attach its conversation or the files it touched to a task by its own call. It only narrows what the host
/// already lets the connection use; the host applies it to the agent's calls and to the person's own alike.
Future<void> showDelegationSetupDialog(BuildContext context, SessionStore store, ChatSession session, DelegateTargets current) =>
    showSelectableDialog<void>(context: context, builder: (_) => DelegationSetupDialog(store: store, session: session, current: current));

/// Asks the host what the session has now, then shows the setup. Nothing opens when the host did not say.
Future<void> openDelegationSetup(BuildContext context, SessionStore store, ChatSession session) async {
  final current = await store.delegateTargets(session);
  if (current == null || !context.mounted) return;
  await showDelegationSetupDialog(context, store, session, current);
}

class DelegationSetupDialog extends StatefulWidget {
  const DelegationSetupDialog({super.key, required this.store, required this.session, required this.current});

  final SessionStore store;
  final ChatSession session;
  final DelegateTargets current;

  @override
  State<DelegationSetupDialog> createState() => _DelegationSetupDialogState();
}

class _DelegationSetupDialogState extends State<DelegationSetupDialog> {
  late final Set<String> _allowed;
  late bool _everyAgent;
  String? _default;
  late final TextEditingController _limit;
  late bool _messages, _files;
  bool _saving = false;
  String? _problem;

  List<(String, String)> get _offered {
    final offered = widget.current.offered;
    return offered.isNotEmpty ? offered : [for (final target in widget.current.targets) (target.source, target.title)];
  }

  @override
  void initState() {
    super.initState();
    final current = widget.current;
    _everyAgent = current.sources == null;
    _allowed = {...?current.sources, if (current.sources == null) for (final (source, _) in _offered) source};
    _default = current.defaultSource;
    _limit = TextEditingController(text: current.maxRunning?.toString() ?? '');
    _messages = current.context.contains('messages');
    _files = current.context.contains('files');
  }

  @override
  void dispose() {
    _limit.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    final text = _limit.text.trim();
    final limit = int.tryParse(text);
    if (text.isNotEmpty && (limit == null || limit < 0)) {
      setState(() => _problem = 'The limit is a number of tasks.');
      return;
    }
    setState(() {
      _saving = true;
      _problem = null;
    });
    final names = [for (final (source, _) in _offered) if (_allowed.contains(source)) source];
    final defaultSource = _default != null && names.contains(_default) ? _default : null;
    final took = await widget.store.delegateSetup(
      widget.session,
      sources: _everyAgent ? null : names,
      defaultSource: defaultSource,
      maxRunning: limit,
      context: [if (_messages) 'messages', if (_files) 'files'],
    );
    if (!mounted) return;
    if (took) {
      Navigator.of(context).pop();
    } else {
      setState(() {
        _saving = false;
        _problem = 'The host did not take the setup.';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final offered = _offered;
    return AlertDialog(
      key: const Key('delegation setup'),
      title: const Text('Delegation setup'),
      content: SizedBox(
        width: 440,
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
            const Text('Let this agent delegate to these Canoa agents:'),
            SwitchListTile(
              key: const Key('delegation setup every'),
              contentPadding: EdgeInsets.zero,
              title: const Text('Every agent this connection may use'),
              value: _everyAgent,
              onChanged: (on) => setState(() {
                _everyAgent = on;
                if (on) _allowed.addAll([for (final (source, _) in offered) source]);
              }),
            ),
            if (!_everyAgent)
              for (final (source, title) in offered)
                CheckboxListTile(
                  key: Key('delegation setup agent $source'),
                  contentPadding: EdgeInsets.zero,
                  dense: true,
                  title: Text(title),
                  value: _allowed.contains(source),
                  onChanged: (on) => setState(() {
                    if (on ?? false) {
                      _allowed.add(source);
                    } else {
                      _allowed.remove(source);
                      if (_default == source) _default = null;
                    }
                  }),
                ),
            DropdownButtonFormField<String?>(
              key: const Key('delegation setup default'),
              isExpanded: true,
              initialValue: _default,
              decoration: const InputDecoration(labelText: 'Agent a task goes to when none is named'),
              items: [
                const DropdownMenuItem<String?>(value: null, child: Text('None: the agent must name one')),
                for (final (source, title) in offered)
                  if (_everyAgent || _allowed.contains(source)) DropdownMenuItem<String?>(value: source, child: Text(title)),
              ],
              onChanged: (source) => setState(() => _default = source),
            ),
            TextField(
              key: const Key('delegation setup limit'),
              controller: _limit,
              keyboardType: TextInputType.number,
              decoration: const InputDecoration(labelText: 'Tasks running at once, at most', helperText: 'Empty leaves the host\'s own limit.'),
            ),
            const SizedBox(height: 8),
            const Text('Let this agent attach, by its own call:'),
            CheckboxListTile(
              key: const Key('delegation setup messages'),
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: const Text('The recent turns of this conversation'),
              value: _messages,
              onChanged: (on) => setState(() => _messages = on ?? false),
            ),
            CheckboxListTile(
              key: const Key('delegation setup files'),
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: const Text('The files it touched'),
              subtitle: const Text('Never sent to another machine. What you start yourself shows you the text first.', style: TextStyle(fontSize: 12)),
              value: _files,
              onChanged: (on) => setState(() => _files = on ?? false),
            ),
            if (_problem != null)
              Padding(padding: const EdgeInsets.only(top: 8), child: Text(_problem!, key: const Key('delegation setup problem'), style: TextStyle(color: Theme.of(context).colorScheme.error))),
          ]),
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Cancel')),
        FilledButton(key: const Key('delegation setup save'), onPressed: _saving ? null : _save, child: const Text('Save')),
      ],
    );
  }
}
