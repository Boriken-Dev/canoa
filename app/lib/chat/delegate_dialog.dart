import 'dart:async';
import 'dart:math';

import 'package:material_ui/material_ui.dart';

import '../acp/delegate_targets.dart';
import '../selectable.dart';
import '../session_store.dart';
import 'delegation_setup_dialog.dart';

/// Asks the person what to hand on and to whom, then has the host start it as a task of [session]: the same task the agent's own
/// `delegate_task` makes, so the agent's `task_status` sees it and the Subagents list shows it. Resolves when the dialog closes.
Future<void> showDelegateDialog(BuildContext context, SessionStore store, ChatSession session) =>
    showSelectableDialog<void>(context: context, builder: (_) => DelegateDialog(store: store, session: session));

/// The choices in the order the person makes them: host, agent, account, model and effort, a profile and its loadout, then where it
/// works, what it is given besides the task (shown in full before it goes) and the task.
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
  String? _account, _model, _effort, _profile, _loadout;

  /// What the agent is given besides the task text: `task` (nothing), `messages` or `files`; and what the host showed of it.
  String _context = 'task';
  ContextPreview? _preview;
  bool _previewing = false;
  String? _previewProblem;
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

  void _clearContext() {
    _context = 'task';
    _preview = null;
    _previewProblem = null;
  }

  /// Asks the host what handing [kind] on would give the agent now, and shows it: what is sent is what the host built for this.
  Future<void> _choosePreview(String kind) async {
    setState(() {
      _context = kind;
      _preview = null;
      _previewProblem = null;
      _edited();
      _previewing = kind != 'task';
    });
    if (kind == 'task') return;
    final preview = await widget.store.delegateContext(widget.session, kind);
    if (!mounted || _context != kind) return;
    setState(() {
      _previewing = false;
      _preview = preview;
      _previewProblem = preview == null ? 'The host has nothing of this conversation to hand over yet.' : null;
      _edited();
    });
  }

  void _pick(DelegateTarget target) {
    _target = target;
    _host = target.hostLabel;
    _account = null;
    _model = null;
    _effort = null;
    _profile = null;
    _loadout = null;
    if (target.remote) _clearContext();
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
    if (_context != 'task' && _preview == null) return false;
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
        profile: target.remote ? null : _profile,
        loadout: target.remote ? null : _loadout,
        context: _context == 'task' ? null : _context,
        contextId: _context == 'task' ? null : _preview?.id,
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
                  if (target != null && target.source == targets.defaultSource)
                    if (targets.suggestedBy('default') case final layer?)
                    Padding(
                      padding: const EdgeInsets.only(top: 4),
                      child: Text('Chosen first because the profile ($layer) suggests it.', key: const Key('delegate default from'), style: const TextStyle(fontSize: 12)),
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
                  if (target != null && !target.remote && target.profiles.isNotEmpty)
                    DropdownButtonFormField<String?>(
                      key: const Key('delegate profile'),
                      initialValue: _profile,
                      decoration: const InputDecoration(labelText: 'Profile'),
                      items: [
                        const DropdownMenuItem<String?>(value: null, child: Text('The agent\'s own')),
                        for (final profile in target.profiles) DropdownMenuItem<String?>(value: profile.name, child: Text(profile.name)),
                      ],
                      onChanged: (profile) => setState(() {
                        _profile = profile;
                        _loadout = null;
                        _edited();
                      }),
                    ),
                  if (target != null && !target.remote && (target.profiles.where((profile) => profile.name == _profile).firstOrNull?.loadouts.isNotEmpty ?? false))
                    DropdownButtonFormField<String?>(
                      key: const Key('delegate loadout'),
                      initialValue: _loadout,
                      decoration: const InputDecoration(labelText: 'Loadout'),
                      items: [
                        const DropdownMenuItem<String?>(value: null, child: Text('The profile\'s own')),
                        for (final loadout in target.profiles.firstWhere((profile) => profile.name == _profile).loadouts) DropdownMenuItem<String?>(value: loadout, child: Text(loadout)),
                      ],
                      onChanged: (loadout) => setState(() {
                        _loadout = loadout;
                        _edited();
                      }),
                    ),
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
                  if (target != null && !target.remote)
                    DropdownButtonFormField<String>(
                      key: const Key('delegate context'),
                      isExpanded: true,
                      initialValue: _context,
                      decoration: const InputDecoration(labelText: 'Give the agent besides the task'),
                      items: const [
                        DropdownMenuItem(value: 'task', child: Text('Nothing: the task text only')),
                        DropdownMenuItem(value: 'messages', child: Text('The recent turns of this conversation')),
                        DropdownMenuItem(value: 'files', child: Text('The files this conversation touched')),
                      ],
                      onChanged: (kind) => unawaited(_choosePreview(kind ?? 'task')),
                    ),
                  if (_previewing) const Padding(padding: EdgeInsets.only(top: 8), child: LinearProgressIndicator(key: Key('delegate context loading'))),
                  if (_previewProblem != null)
                    Padding(padding: const EdgeInsets.only(top: 8), child: Text(_previewProblem!, key: const Key('delegate context none'), style: TextStyle(color: Theme.of(context).colorScheme.error))),
                  if (_preview != null) _ContextPreviewBox(_preview!),
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
        if (targets != null && targets.targets.isNotEmpty)
          TextButton(key: const Key('delegate setup open'), onPressed: () => unawaited(showDelegationSetupDialog(context, widget.store, widget.session, targets)), child: const Text('Delegation setup')),
        TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Cancel')),
        FilledButton(key: const Key('delegate send'), onPressed: _ready ? _send : null, child: Text(_uncertain ? 'Send again' : 'Delegate')),
      ],
    );
  }
}

/// What the host will give the agent besides the task, in full, before anything is sent: each turn or file with its size, what was
/// left out, and the exact text.
class _ContextPreviewBox extends StatelessWidget {
  const _ContextPreviewBox(this.preview);

  final ContextPreview preview;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      key: const Key('delegate context preview'),
      margin: const EdgeInsets.only(top: 8),
      padding: const EdgeInsets.all(8),
      decoration: BoxDecoration(border: Border.all(color: scheme.outlineVariant), borderRadius: BorderRadius.circular(6)),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text('The agent is given exactly this, before the task:', style: Theme.of(context).textTheme.labelMedium),
        const SizedBox(height: 4),
        for (final (name, bytes) in preview.items) Text('$name  ($bytes bytes)', style: const TextStyle(fontSize: 12)),
        for (final line in preview.skipped) Text('Left out: $line', style: TextStyle(fontSize: 12, color: scheme.error)),
        if (preview.truncated) const Text('Cut to fit the bound the host sets.', style: TextStyle(fontSize: 12)),
        ExpansionTile(
          key: const Key('delegate context text'),
          tilePadding: EdgeInsets.zero,
          title: const Text('The text', style: TextStyle(fontSize: 13)),
          children: [ConstrainedBox(constraints: const BoxConstraints(maxHeight: 220), child: SingleChildScrollView(child: SelectableText(preview.text, style: const TextStyle(fontSize: 12))))],
        ),
      ]),
    );
  }
}
