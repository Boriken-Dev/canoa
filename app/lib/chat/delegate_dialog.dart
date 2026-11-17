import 'dart:math';

import 'package:flutter/material.dart';

import '../acp/delegate_targets.dart';
import '../selectable.dart';
import '../session_store.dart';

/// Asks the person what to hand on and to whom, then has the host start it as a task of [session]: the same task the agent's own
/// `delegate_task` makes, so the agent's `task_status` sees it and the Subagents list shows it. Resolves when the dialog closes.
Future<void> showDelegateDialog(BuildContext context, SessionStore store, ChatSession session) =>
    showSelectableDialog<void>(context: context, builder: (_) => DelegateDialog(store: store, session: session));

/// The choices in the order the person makes them: host, agent, account, model and effort, then where it works and the task.
class DelegateDialog extends StatefulWidget {
  const DelegateDialog({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  State<DelegateDialog> createState() => _DelegateDialogState();
}

class _DelegateDialogState extends State<DelegateDialog> {
  static const _anyHost = 'This host', _otherHost = 'Another machine';
  static final _random = Random.secure();

  DelegateTargets? _targets;
  bool _loading = true, _sending = false;
  String _host = _anyHost;
  DelegateTarget? _target;
  String? _account, _model, _effort;
  final _task = TextEditingController(), _title = TextEditingController(), _folder = TextEditingController();

  /// Names the request being sent: kept while it is sent again unchanged, so the host never starts it twice.
  String _requestId = _newId();

  /// What the host last said of the request: a refusal, or that it did not answer.
  String? _problem;
  bool _uncertain = false;

  static String _newId() => 'r-${[for (var i = 0; i < 8; i++) _random.nextInt(256).toRadixString(16).padLeft(2, '0')].join()}';

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _task.dispose();
    _title.dispose();
    _folder.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final targets = await widget.store.delegateTargets(widget.session);
    if (!mounted) return;
    setState(() {
      _targets = targets;
      _loading = false;
      final all = targets?.targets ?? const <DelegateTarget>[];
      final preferred = all.where((target) => target.source == targets?.defaultSource).firstOrNull ?? all.firstOrNull;
      if (preferred != null) _pick(preferred);
    });
  }

  List<DelegateTarget> get _shown => [for (final target in _targets?.targets ?? const <DelegateTarget>[]) if (target.hostLabel == _host) target];

  void _edited() => _requestId = _newId();

  void _pick(DelegateTarget target) {
    _target = target;
    _host = target.hostLabel;
    _account = null;
    _model = null;
    _effort = null;
    _problem = null;
    _uncertain = false;
    _edited();
    if (!target.remote) _loadModels(target);
  }

  Future<void> _loadModels(DelegateTarget target) async {
    final listed = await widget.store.delegateTargets(widget.session, source: target.source);
    final withModels = listed?.targets.where((each) => each.source == target.source).firstOrNull;
    if (!mounted || withModels == null || _target?.source != target.source) return;
    setState(() => _target = withModels);
  }

  List<String> get _efforts {
    final target = _target;
    if (target == null) return const [];
    final chosen = target.models.where((model) => model.id == _model).firstOrNull;
    return chosen?.efforts ?? {for (final model in target.models) ...model.efforts}.toList();
  }

  bool get _ready {
    final target = _target;
    if (target == null || _task.text.trim().isEmpty || _sending) return false;
    return !target.remote || _folder.text.trim().isNotEmpty;
  }

  Future<void> _send() async {
    final target = _target;
    if (target == null) return;
    final folder = _folder.text.trim();
    final title = _title.text.trim();
    setState(() {
      _sending = true;
      _problem = null;
      _uncertain = false;
    });
    final outcome = await widget.store.delegateTask(
      widget.session,
      DelegateRequest(
        requestId: _requestId,
        source: target.source,
        task: _task.text,
        title: title.isEmpty ? null : title,
        folder: !target.remote && folder.isNotEmpty ? folder : null,
        workspace: target.remote ? folder : null,
        model: target.remote ? null : _model,
        effort: target.remote ? null : _effort,
        account: target.remote ? null : _account,
      ),
    );
    if (!mounted) return;
    switch (outcome) {
      case DelegateStarted():
        Navigator.of(context).pop();
      case DelegateRefused(:final message):
        setState(() {
          _sending = false;
          _problem = message;
        });
      case DelegateUncertain():
        setState(() {
          _sending = false;
          _uncertain = true;
        });
    }
  }

  @override
  Widget build(BuildContext context) {
    final targets = _targets;
    final target = _target;
    return AlertDialog(
      key: const Key('delegate dialog'),
      title: const Text('Delegate task'),
      content: SizedBox(
        width: 460,
        child: _loading
            ? const Padding(padding: EdgeInsets.all(24), child: Center(child: CircularProgressIndicator()))
            : targets == null || targets.targets.isEmpty
            ? Text(targets == null ? 'The host did not say what it can hand a task to.' : 'This conversation may not hand a task to any agent.', key: const Key('delegate none'))
            : SingleChildScrollView(
                child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                  if (targets.targets.any((each) => each.remote) && targets.targets.any((each) => !each.remote))
                    DropdownButtonFormField<String>(
                      key: const Key('delegate host'),
                      initialValue: _host,
                      decoration: const InputDecoration(labelText: 'Host'),
                      items: const [DropdownMenuItem(value: _anyHost, child: Text(_anyHost)), DropdownMenuItem(value: _otherHost, child: Text(_otherHost))],
                      onChanged: (host) => setState(() {
                        _host = host ?? _host;
                        final first = _shown.firstOrNull;
                        if (first != null) _pick(first);
                      }),
                    ),
                  DropdownButtonFormField<String>(
                    key: const Key('delegate agent'),
                    initialValue: target?.source,
                    decoration: const InputDecoration(labelText: 'Agent'),
                    items: [
                      for (final each in _shown)
                        DropdownMenuItem(value: each.source, child: Text(each.ready ? each.title : '${each.title} (not ready)')),
                    ],
                    onChanged: (source) => setState(() {
                      final next = _shown.where((each) => each.source == source).firstOrNull;
                      if (next != null) _pick(next);
                    }),
                  ),
                  if (target != null && !target.ready)
                    Padding(
                      padding: const EdgeInsets.only(top: 4),
                      child: Text('Not ready: ${target.missing.join(', ')} is missing.', key: const Key('delegate not ready'), style: TextStyle(color: Theme.of(context).colorScheme.error, fontSize: 12)),
                    ),
                  if (target != null && !target.remote && target.accounts.isNotEmpty)
                    DropdownButtonFormField<String?>(
                      key: const Key('delegate account'),
                      initialValue: _account,
                      decoration: const InputDecoration(labelText: 'Account'),
                      items: [
                        const DropdownMenuItem<String?>(value: null, child: Text('The agent\'s own')),
                        for (final account in target.accounts) DropdownMenuItem<String?>(value: account, child: Text(account)),
                      ],
                      onChanged: (account) => setState(() {
                        _account = account;
                        _edited();
                      }),
                    ),
                  if (target != null && !target.remote && target.models.isNotEmpty) ...[
                    DropdownButtonFormField<String?>(
                      key: const Key('delegate model'),
                      initialValue: _model,
                      decoration: const InputDecoration(labelText: 'Model'),
                      items: [
                        const DropdownMenuItem<String?>(value: null, child: Text('The agent\'s own')),
                        for (final model in target.models) DropdownMenuItem<String?>(value: model.id, child: Text(model.name)),
                      ],
                      onChanged: (model) => setState(() {
                        _model = model;
                        if (!_efforts.contains(_effort)) _effort = null;
                        _edited();
                      }),
                    ),
                    if (_efforts.isNotEmpty)
                      DropdownButtonFormField<String?>(
                        key: const Key('delegate effort'),
                        initialValue: _effort,
                        decoration: const InputDecoration(labelText: 'Effort'),
                        items: [
                          const DropdownMenuItem<String?>(value: null, child: Text('The agent\'s own')),
                          for (final effort in _efforts) DropdownMenuItem<String?>(value: effort, child: Text(effort)),
                        ],
                        onChanged: (effort) => setState(() {
                          _effort = effort;
                          _edited();
                        }),
                      ),
                  ],
                  TextField(
                    key: const Key('delegate folder'),
                    controller: _folder,
                    decoration: InputDecoration(
                      labelText: target?.remote == true ? 'Folder on that machine' : 'Folder (optional)',
                      helperText: target?.remote == true
                          ? 'A folder of the other machine, which checks it. This conversation\'s own folder is never sent.'
                          : 'This conversation\'s folder, or one below it.',
                      helperMaxLines: 2,
                    ),
                    onChanged: (_) => setState(_edited),
                  ),
                  TextField(
                    key: const Key('delegate title'),
                    controller: _title,
                    decoration: const InputDecoration(labelText: 'Name (optional)'),
                    onChanged: (_) => setState(_edited),
                  ),
                  TextField(
                    key: const Key('delegate task field'),
                    controller: _task,
                    minLines: 3,
                    maxLines: 8,
                    decoration: const InputDecoration(labelText: 'Task', helperText: 'The agent sees only this text.'),
                    onChanged: (_) => setState(_edited),
                  ),
                  if (_problem != null)
                    Padding(
                      padding: const EdgeInsets.only(top: 8),
                      child: Text(_problem!, key: const Key('delegate refused'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
                    ),
                  if (_uncertain)
                    const Padding(
                      padding: EdgeInsets.only(top: 8),
                      child: Text(
                        'The host did not answer, so the task may or may not have started. Look in the Subagents list before sending it again: it is not sent again by itself, and sending it again unchanged will not start it twice.',
                        key: Key('delegate uncertain'),
                      ),
                    ),
                ]),
              ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Cancel')),
        FilledButton(key: const Key('delegate send'), onPressed: _ready ? _send : null, child: Text(_uncertain ? 'Send again' : 'Delegate')),
      ],
    );
  }
}
