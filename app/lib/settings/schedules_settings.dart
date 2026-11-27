import 'dart:async';

import 'package:flutter/services.dart' show FilteringTextInputFormatter;
import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../l10n/app_text.dart';
import 'settings_row.dart';

/// What the app asks the host with: `schedules/list`, `schedules/set`, `schedules/remove` and `schedules/run`.
typedef ScheduleCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// The names `[schedules.ID] weekdays` takes, from Sunday.
const List<String> weekdayKeys = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];

/// What became of a task's last run.
final class TaskRun {
  const TaskRun({required this.at, required this.result, this.note = '', this.session});

  final DateTime at;

  /// `completed`, `needs_you`, `failed`, `stopped` or `missed`.
  final String result;
  final String note;

  /// The agent's id for the session, which finds it among the sessions the host kept.
  final String? session;

  static TaskRun? read(Object? json) {
    if (json is! Map || json['at'] is! int || json['result'] is! String) return null;
    return TaskRun(
      at: DateTime.fromMillisecondsSinceEpoch(json['at']! as int),
      result: json['result']! as String,
      note: json['note'] is String ? json['note']! as String : '',
      session: json['session'] is String ? json['session'] as String : null,
    );
  }
}

/// One prompt the host runs on a schedule, as `schedules/list` words it.
final class ScheduledTask {
  const ScheduledTask({
    required this.id,
    this.title,
    required this.entry,
    required this.folder,
    required this.prompt,
    required this.repeat,
    required this.at,
    this.date,
    this.weekdays = const [],
    this.timeZone,
    this.enabled = true,
    this.mode,
    this.timeoutMinutes = 120,
    this.next,
    this.running = false,
    this.last,
  });

  final String id;
  final String? title;
  final String entry, folder, prompt;

  /// `once`, `daily` or `weekly`.
  final String repeat;

  /// The time of day, `HH:MM`, and with `once` the day, `YYYY-MM-DD`, in [timeZone].
  final String at;
  final String? date;

  /// The days of a weekly task, from 0 for Sunday.
  final List<int> weekdays;

  /// The IANA zone the times are read in; null reads them in the host's computer's own.
  final String? timeZone;
  final bool enabled;
  final String? mode;
  final int timeoutMinutes;

  /// When it runs next; null when none comes (it is off, or a one-time task that has run).
  final DateTime? next;
  final bool running;
  final TaskRun? last;

  /// What it is called: its title, else its id.
  String get name => title ?? id;

  static ScheduledTask? read(Object? json) {
    if (json is! Map || json['id'] is! String || json['entry'] is! String || json['folder'] is! String || json['prompt'] is! String) return null;
    return ScheduledTask(
      id: json['id']! as String,
      title: json['title'] is String ? json['title'] as String : null,
      entry: json['entry']! as String,
      folder: json['folder']! as String,
      prompt: json['prompt']! as String,
      repeat: json['repeat'] is String ? json['repeat']! as String : 'daily',
      at: json['at'] is String ? json['at']! as String : '00:00',
      date: json['date'] is String ? json['date'] as String : null,
      weekdays: [if (json['weekdays'] is List) for (final day in json['weekdays'] as List) if (day is int) day],
      timeZone: json['time_zone'] is String ? json['time_zone'] as String : null,
      enabled: json['enabled'] != false,
      mode: json['mode'] is String ? json['mode'] as String : null,
      timeoutMinutes: json['timeout_minutes'] is int ? json['timeout_minutes']! as int : 120,
      next: json['next'] is int ? DateTime.fromMillisecondsSinceEpoch(json['next']! as int) : null,
      running: json['running'] == true,
      last: TaskRun.read(json['last']),
    );
  }

  /// The table `schedules/set` writes for it.
  Map<String, Object?> toTable({bool? enabled}) => {
    'entry': entry,
    'folder': folder,
    'prompt': prompt,
    'title': ?title,
    'repeat': repeat,
    'at': at,
    'date': ?date,
    if (repeat == 'weekly') 'weekdays': [for (final day in weekdays) weekdayKeys[day]],
    'time_zone': ?timeZone,
    'enabled': enabled ?? this.enabled,
    'mode': ?mode,
    'timeout_minutes': timeoutMinutes,
  };
}

/// The host's scheduled tasks (`[schedules.ID]`): listed with when each runs next and what became of its last run, added, removed,
/// switched off and run at once. The host runs them, so they run with this application closed.
class ScheduledTasks extends ChangeNotifier {
  ScheduledTasks(this._call);

  final ScheduleCall _call;

  /// Says a word of this page in the person's language; the page sets it, and the base English is used before it does.
  String Function(String key, [Map<String, Object?> args]) say = AppText.base.call;

  bool _disposed = false;
  List<ScheduledTask> _tasks = const [];
  List<String> _entries = const [];
  String? _problem;
  bool _loading = false, _loaded = false;

  List<ScheduledTask> get tasks => _tasks;

  /// The agent entries a task may run on, as the host serves them.
  List<String> get entries => _entries;

  /// Why the list could not be read or the last change was refused, in the host's words.
  String? get problem => _problem;
  bool get loading => _loading;
  bool get loaded => _loaded;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Reads the tasks from the host.
  Future<void> load() async {
    _loading = true;
    notifyListeners();
    try {
      final answer = await _call('schedules/list', const {});
      final listed = answer is Map ? answer : const {};
      _tasks = [if (listed['tasks'] is List) for (final each in listed['tasks'] as List) ?ScheduledTask.read(each)];
      _entries = [if (listed['entries'] is List) for (final each in listed['entries'] as List) if (each is String) each];
      _problem = null;
      _loaded = true;
    } on ControlFailed catch (error) {
      _problem = error.code == -32001 ? say('schedules.denied') : error.message;
    } on Object catch (error) {
      _problem = say('schedules.unreachable', {'why': '$error'});
    } finally {
      _loading = false;
      notifyListeners();
    }
  }

  /// The id a task called [name] is kept under: its lower-case words joined with `-`, made new among the tasks held.
  String idFor(String name) {
    final words = name.toLowerCase().replaceAll(RegExp('[^a-z0-9]+'), '-').replaceAll(RegExp(r'^-+|-+$'), '');
    final base = words.isEmpty ? 'task' : (words.length > 28 ? words.substring(0, 28) : words);
    var id = base, n = 2;
    while (_tasks.any((task) => task.id == id)) {
      id = '$base-${n++}';
    }
    return id;
  }

  /// Makes or replaces the task [id]; returns what is wrong in the host's words, or null once canoa.cfg holds it.
  Future<String?> save(String id, Map<String, Object?> table) => _ask('schedules/set', {'id': id, 'task': table});

  Future<String?> remove(String id) => _ask('schedules/remove', {'id': id});

  /// Starts the task now, whatever its time.
  Future<String?> runNow(String id) => _ask('schedules/run', {'id': id});

  Future<String?> _ask(String method, Map<String, Object?> params) async {
    try {
      await _call(method, params);
      _problem = null;
      await load();
      return null;
    } on ControlFailed catch (error) {
      return error.code == -32001 ? say('schedules.denied') : error.message;
    } on Object catch (error) {
      return say('schedules.unreachable', {'why': '$error'});
    }
  }
}

/// Settings' Scheduled tasks page: the host's prompts that run on a schedule, with the next run and the last result of each.
class SchedulesSection extends StatefulWidget {
  const SchedulesSection({super.key, required this.tasks, this.folders = const []});

  final ScheduledTasks tasks;

  /// Folders to offer when a task is added, such as the ones sessions were opened in.
  final List<String> folders;

  @override
  State<SchedulesSection> createState() => _SchedulesSectionState();
}

class _SchedulesSectionState extends State<SchedulesSection> {
  ScheduledTasks get tasks => widget.tasks;
  Timer? _refresh;

  @override
  void initState() {
    super.initState();
    unawaited(tasks.load());
    // The host runs the tasks while this is open: what became of one shows without asking again.
    _refresh = Timer.periodic(const Duration(seconds: 10), (_) => unawaited(tasks.load()));
  }

  @override
  void dispose() {
    _refresh?.cancel();
    super.dispose();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    tasks.say = context.t;
  }

  void _tell(String? problem) {
    if (problem == null || !mounted) return;
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(problem)));
  }

  Future<void> _add() async {
    final made = await showDialog<_NewTask>(
      context: context,
      builder: (context) => _AddTaskDialog(entries: tasks.entries, folders: widget.folders),
    );
    if (made == null) return;
    _tell(await tasks.save(tasks.idFor(made.name), made.table));
  }

  Future<void> _remove(ScheduledTask task) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.t('schedules.remove_title', {'name': task.name})),
        content: Text(context.t('schedules.remove_body')),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: Text(context.t('schedules.cancel'))),
          FilledButton(key: const Key('schedule-remove-confirm'), onPressed: () => Navigator.pop(context, true), child: Text(context.t('schedules.remove'))),
        ],
      ),
    );
    if (sure ?? false) _tell(await tasks.remove(task.id));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: tasks,
    builder: (context, _) {
      final scheme = Theme.of(context).colorScheme;
      return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(context.t('schedules.intro'), style: Theme.of(context).textTheme.bodyMedium?.copyWith(color: scheme.onSurfaceVariant)),
        const SizedBox(height: 12),
        if (tasks.problem case final problem?)
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: Text(problem, key: const Key('schedules-problem'), style: TextStyle(color: scheme.error)),
          ),
        if (tasks.loading && !tasks.loaded) const Padding(padding: EdgeInsets.all(12), child: LinearProgressIndicator()),
        if (tasks.loaded) ...[
          if (tasks.tasks.isEmpty)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 8),
              child: Text(context.t('schedules.empty'), key: const Key('schedules-empty')),
            )
          else
            SettingsGroup(children: [for (final task in tasks.tasks) _row(context, task)]),
          Align(
            alignment: Alignment.centerLeft,
            child: FilledButton.icon(
              key: const Key('schedule-add'),
              onPressed: _add,
              icon: const Icon(Icons.add),
              label: Text(context.t('schedules.add')),
            ),
          ),
        ],
      ]);
    },
  );

  Widget _row(BuildContext context, ScheduledTask task) {
    final scheme = Theme.of(context).colorScheme;
    final muted = TextStyle(color: scheme.onSurfaceVariant, fontSize: 12);
    final last = task.last;
    return Padding(
      key: Key('schedule-${task.id}'),
      padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Row(children: [
          Expanded(child: Text(task.name, style: Theme.of(context).textTheme.titleSmall)),
          Switch(
            key: Key('schedule-enabled-${task.id}'),
            value: task.enabled,
            onChanged: (on) async => _tell(await tasks.save(task.id, task.toTable(enabled: on))),
          ),
          IconButton(
            key: Key('schedule-run-${task.id}'),
            tooltip: context.t('schedules.run_now'),
            icon: const Icon(Icons.play_arrow),
            onPressed: task.running ? null : () async => _tell(await tasks.runNow(task.id)),
          ),
          IconButton(key: Key('schedule-delete-${task.id}'), tooltip: context.t('schedules.remove'), icon: const Icon(Icons.delete_outline), onPressed: () => _remove(task)),
        ]),
        Text(_when(context, task), style: muted),
        Text(context.t('schedules.where', {'entry': task.entry, 'folder': task.folder}), style: muted, maxLines: 1, overflow: TextOverflow.ellipsis),
        const SizedBox(height: 4),
        Text(task.prompt, maxLines: 2, overflow: TextOverflow.ellipsis),
        const SizedBox(height: 4),
        Text(
          key: Key('schedule-next-${task.id}'),
          task.running
              ? context.t('schedules.running')
              : task.next == null
              ? context.t('schedules.next_none')
              : context.t('schedules.next', {'when': _stamp(context, task.next!)}),
          style: muted,
        ),
        Text(
          key: Key('schedule-last-${task.id}'),
          last == null
              ? context.t('schedules.never')
              : context.t('schedules.last', {
                  'result': context.t('schedules.result.${last.result}'),
                  'when': _stamp(context, last.at),
                  'note': last.note.isEmpty ? '' : ': ${last.note}',
                }),
          style: muted.copyWith(color: last != null && (last.result == 'failed') ? scheme.error : null),
        ),
      ]),
    );
  }

  /// "Every day at 09:30", "Mon, Thu at 08:15" or "2 Dec 2026 at 09:30".
  static String _when(BuildContext context, ScheduledTask task) {
    final time = task.at;
    return switch (task.repeat) {
      'once' => context.t('schedules.at_once', {'date': task.date ?? '', 'time': time}),
      'weekly' => context.t('schedules.at_weekly', {'days': [for (final day in task.weekdays) context.t('schedules.day.$day')].join(', '), 'time': time}),
      _ => context.t('schedules.at_daily', {'time': time}),
    };
  }

  static String _stamp(BuildContext context, DateTime at) {
    final local = MaterialLocalizations.of(context);
    return '${local.formatMediumDate(at)} ${local.formatTimeOfDay(TimeOfDay.fromDateTime(at), alwaysUse24HourFormat: true)}';
  }
}

/// What the dialog that adds a task gives back.
final class _NewTask {
  const _NewTask(this.name, this.table);

  final String name;
  final Map<String, Object?> table;
}

class _AddTaskDialog extends StatefulWidget {
  const _AddTaskDialog({required this.entries, required this.folders});

  final List<String> entries, folders;

  @override
  State<_AddTaskDialog> createState() => _AddTaskDialogState();
}

class _AddTaskDialogState extends State<_AddTaskDialog> {
  final _name = TextEditingController();
  final _folder = TextEditingController();
  final _prompt = TextEditingController();
  final _time = TextEditingController(text: '09:00');
  final _date = TextEditingController();
  String? _entry;
  String _repeat = 'daily';
  final _days = <int>{1};
  String? _wrong;

  @override
  void initState() {
    super.initState();
    _entry = widget.entries.firstOrNull;
    if (widget.folders.isNotEmpty) _folder.text = widget.folders.first;
    final tomorrow = DateTime.now().add(const Duration(days: 1));
    _date.text = '${tomorrow.year.toString().padLeft(4, '0')}-${tomorrow.month.toString().padLeft(2, '0')}-${tomorrow.day.toString().padLeft(2, '0')}';
  }

  @override
  void dispose() {
    for (final controller in [_name, _folder, _prompt, _time, _date]) {
      controller.dispose();
    }
    super.dispose();
  }

  void _save() {
    final time = RegExp(r'^([01]?\d|2[0-3]):([0-5]\d)$').firstMatch(_time.text.trim());
    final date = RegExp(r'^\d{4}-\d{2}-\d{2}$').hasMatch(_date.text.trim());
    final why = _entry == null
        ? context.t('schedules.need_agent')
        : _folder.text.trim().isEmpty || _prompt.text.trim().isEmpty
        ? context.t('schedules.need_text')
        : time == null
        ? context.t('schedules.need_time')
        : _repeat == 'once' && !date
        ? context.t('schedules.need_date')
        : _repeat == 'weekly' && _days.isEmpty
        ? context.t('schedules.need_days')
        : null;
    if (why != null) return setState(() => _wrong = why);
    final hour = time!.group(1)!.padLeft(2, '0');
    final title = _name.text.trim();
    Navigator.pop(
      context,
      _NewTask(
        title.isEmpty ? _prompt.text.trim().split('\n').first : title,
        {
          'entry': _entry,
          'folder': _folder.text.trim(),
          'prompt': _prompt.text.trim(),
          if (title.isNotEmpty) 'title': title,
          'repeat': _repeat,
          'at': '$hour:${time.group(2)}',
          if (_repeat == 'once') 'date': _date.text.trim(),
          if (_repeat == 'weekly') 'weekdays': [for (final day in _days.toList()..sort()) weekdayKeys[day]],
        },
      ),
    );
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(context.t('schedules.add_title')),
    content: SizedBox(
      width: 440,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          TextField(key: const Key('schedule-name'), controller: _name, decoration: InputDecoration(labelText: context.t('schedules.name'))),
          const SizedBox(height: 8),
          DropdownButtonFormField<String>(
            key: const Key('schedule-agent'),
            initialValue: _entry,
            decoration: InputDecoration(labelText: context.t('schedules.agent')),
            items: [for (final entry in widget.entries) DropdownMenuItem(value: entry, child: Text(entry))],
            onChanged: (value) => setState(() => _entry = value),
          ),
          const SizedBox(height: 8),
          TextField(key: const Key('schedule-folder'), controller: _folder, decoration: InputDecoration(labelText: context.t('schedules.folder'))),
          const SizedBox(height: 8),
          TextField(
            key: const Key('schedule-prompt'),
            controller: _prompt,
            minLines: 3,
            maxLines: 8,
            decoration: InputDecoration(labelText: context.t('schedules.prompt')),
          ),
          const SizedBox(height: 8),
          DropdownButtonFormField<String>(
            key: const Key('schedule-repeat'),
            initialValue: _repeat,
            decoration: InputDecoration(labelText: context.t('schedules.repeat')),
            items: [for (final each in const ['once', 'daily', 'weekly']) DropdownMenuItem(value: each, child: Text(context.t('schedules.$each')))],
            onChanged: (value) => setState(() => _repeat = value ?? 'daily'),
          ),
          const SizedBox(height: 8),
          TextField(
            key: const Key('schedule-time'),
            controller: _time,
            keyboardType: TextInputType.datetime,
            inputFormatters: [FilteringTextInputFormatter.allow(RegExp('[0-9:]'))],
            decoration: InputDecoration(labelText: context.t('schedules.time'), hintText: '09:30'),
          ),
          if (_repeat == 'once') ...[
            const SizedBox(height: 8),
            TextField(key: const Key('schedule-date'), controller: _date, decoration: InputDecoration(labelText: context.t('schedules.date'), hintText: '2026-12-01')),
          ],
          if (_repeat == 'weekly') ...[
            const SizedBox(height: 8),
            Wrap(spacing: 6, children: [
              for (var day = 0; day < 7; day++)
                FilterChip(
                  key: Key('schedule-day-$day'),
                  label: Text(context.t('schedules.day.$day')),
                  selected: _days.contains(day),
                  onSelected: (on) => setState(() => on ? _days.add(day) : _days.remove(day)),
                ),
            ]),
          ],
          if (_wrong != null)
            Padding(padding: const EdgeInsets.only(top: 8), child: Text(_wrong!, key: const Key('schedule-wrong'), style: TextStyle(color: Theme.of(context).colorScheme.error))),
        ]),
      ),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: Text(context.t('schedules.cancel'))),
      FilledButton(key: const Key('schedule-save'), onPressed: _save, child: Text(context.t('schedules.save'))),
    ],
  );
}
