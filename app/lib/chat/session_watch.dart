import 'dart:async' show unawaited;

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:flutter/material.dart';

import '../desktop/control_client.dart' show hostWords;
import '../l10n/app_text.dart';

/// A watch the host keeps on a shared session, as `sessions/activity` and `sessions/watch` describe it.
final class SessionWatchStatus {
  const SessionWatchStatus({
    required this.prompt,
    this.everyMs,
    this.cron,
    this.maxPrompts = 5,
    this.promptsSent = 0,
    this.whenIdle = true,
    this.quietMs,
    this.whileChildren = false,
    this.cpuBelowMs,
    this.memoryNotShrinking = false,
    this.stopOnInput = false,
    this.nextCheckAt,
    this.lastCheckAt,
    this.lastPrompted = false,
    this.lastWhy,
    this.stopped,
  });

  final String prompt;
  final int? everyMs;
  final String? cron;

  /// Prompts sent between two of the person's messages before it holds off; 0 is no stop.
  final int maxPrompts;
  final int promptsSent;
  final bool whenIdle;
  final int? quietMs;
  final bool whileChildren;
  final int? cpuBelowMs;
  final bool memoryNotShrinking;
  final bool stopOnInput;

  /// When it checks next; null once it stopped.
  final DateTime? nextCheckAt;
  final DateTime? lastCheckAt;
  final bool lastPrompted;

  /// Why the last check sent nothing.
  final String? lastWhy;

  /// Why it stopped by itself.
  final String? stopped;

  bool get active => stopped == null && nextCheckAt != null;

  static SessionWatchStatus? parse(Object? json) {
    if (json is! Map) return null;
    int? number(Object? value) => value is num ? value.toInt() : null;
    DateTime? time(Object? value) => value is num ? DateTime.fromMillisecondsSinceEpoch(value.toInt()) : null;
    String? text(Object? value) => value is String && value.isNotEmpty ? value : null;
    final schedule = json['schedule'] is Map ? json['schedule'] as Map : const {};
    final last = json['lastCheck'] is Map ? json['lastCheck'] as Map : null;
    return SessionWatchStatus(
      prompt: text(json['prompt']) ?? '',
      everyMs: number(schedule['everyMs']),
      cron: text(schedule['cron']),
      maxPrompts: number(json['maxPrompts']) ?? 5,
      promptsSent: number(json['promptsSent']) ?? 0,
      whenIdle: json['whenIdle'] != false,
      quietMs: number(json['quietMs']),
      whileChildren: json['whileChildren'] == true,
      cpuBelowMs: number(json['cpuBelowMs']),
      memoryNotShrinking: json['memoryNotShrinking'] == true,
      stopOnInput: json['stopOnInput'] == true,
      nextCheckAt: time(json['nextCheckAt']),
      lastCheckAt: time(last?['at']),
      lastPrompted: last?['prompted'] == true,
      lastWhy: text(last?['why']),
      stopped: text(json['stopped']),
    );
  }
}

/// What the host says of a shared session now: how long it has been quiet, what its agent's processes use, and its watch.
final class SessionActivity {
  const SessionActivity({this.quietMs, this.processes, this.cpuMs, this.memoryBytes, this.watch});

  final int? quietMs;

  /// How many processes the agent runs, itself included; null when it is no process of the host's computer.
  final int? processes;
  final int? cpuMs;
  final int? memoryBytes;
  final SessionWatchStatus? watch;

  static SessionActivity? parse(Object? json) {
    if (json is! Map) return null;
    int? number(Object? value) => value is num ? value.toInt() : null;
    final processes = json['processes'] is Map ? json['processes'] as Map : const {};
    return SessionActivity(
      quietMs: number(json['quietMs']),
      processes: number(processes['count']),
      cpuMs: number(processes['cpuMs']),
      memoryBytes: number(processes['memoryBytes']),
      watch: SessionWatchStatus.parse(json['watch']),
    );
  }
}

/// The host's watches of its shared sessions, through its control channel: `sessions/activity` and `sessions/watch`.
final class SessionWatches {
  const SessionWatches(this._call);

  final Future<Object?> Function(String method, Map<String, Object?> params) _call;

  /// What the host says of [sessionId] now; null when it runs no shared session by that id this key may attach to.
  Future<SessionActivity?> activity(String sessionId) async {
    final answer = await _call('sessions/activity', {'session': sessionId});
    final sessions = answer is Map ? answer['sessions'] : null;
    return sessions is List && sessions.isNotEmpty ? SessionActivity.parse(sessions.first) : null;
  }

  /// Arms or changes the watch of the session [params] name, or takes it away with `enabled: false`; the watch as the host now
  /// holds it, or null when there is none. Throws the host's refusal.
  Future<SessionWatchStatus?> configure(Map<String, Object?> params) async {
    final answer = await _call('sessions/watch', params);
    return answer is Map ? SessionWatchStatus.parse(answer['watch']) : null;
  }
}

/// The Watch section of a shared session's details: what the host measures of it, and a watch that prompts it again on a
/// schedule when the conditions the person picks hold.
class SessionWatchCard extends StatefulWidget {
  const SessionWatchCard({super.key, required this.sessionId, required this.watches, this.utcOffset});

  final String sessionId;
  final SessionWatches watches;

  /// The clock a cron line is read on; this computer's when null.
  final Duration? utcOffset;

  @override
  State<SessionWatchCard> createState() => _SessionWatchCardState();
}

enum _Prompt { proceed, status, custom }

class _SessionWatchCardState extends State<SessionWatchCard> {
  SessionActivity? _activity;
  String? _problem;
  bool _busy = false;
  _Prompt _prompt = _Prompt.proceed;
  bool _cron = false;
  bool _whenIdle = true, _whileChildren = false, _memoryNotShrinking = false, _stopOnInput = false;
  final _custom = TextEditingController();
  final _every = TextEditingController(text: '60');
  final _cronLine = TextEditingController(text: '*/5 * * * *');
  final _quiet = TextEditingController(text: '30');
  final _cpuBelow = TextEditingController();
  final _maxPrompts = TextEditingController(text: '5');

  @override
  void initState() {
    super.initState();
    unawaited(_refresh());
  }

  @override
  void didUpdateWidget(covariant SessionWatchCard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.sessionId != widget.sessionId) unawaited(_refresh());
  }

  @override
  void dispose() {
    for (final controller in [_custom, _every, _cronLine, _quiet, _cpuBelow, _maxPrompts]) {
      controller.dispose();
    }
    super.dispose();
  }

  Future<void> _refresh() async {
    try {
      final activity = await widget.watches.activity(widget.sessionId);
      if (mounted) {
        setState(() {
          _activity = activity;
          _problem = null;
        });
      }
    } on Object catch (error) {
      if (mounted) setState(() => _problem = hostWords(error));
    }
  }

  String _promptText(BuildContext context) => switch (_prompt) {
    _Prompt.proceed => context.t('watch.prompt.continue'),
    _Prompt.status => context.t('watch.prompt.status'),
    _Prompt.custom => _custom.text.trim(),
  };

  /// A whole number of 0 or more in [controller], times [unit]; null when it is empty.
  static int? _whole(TextEditingController controller, {int unit = 1}) {
    final value = int.tryParse(controller.text.trim());
    return value == null || value < 0 ? null : value * unit;
  }

  Future<void> _configure(Map<String, Object?> params) async {
    setState(() {
      _busy = true;
      _problem = null;
    });
    try {
      final watch = await widget.watches.configure({'session': widget.sessionId, ...params});
      if (!mounted) return;
      setState(() => _activity = SessionActivity(
        quietMs: _activity?.quietMs,
        processes: _activity?.processes,
        cpuMs: _activity?.cpuMs,
        memoryBytes: _activity?.memoryBytes,
        watch: watch,
      ));
    } on Object catch (error) {
      if (mounted) setState(() => _problem = hostWords(error));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _arm(BuildContext context) => _configure({
    if (_cron) ...{
      'cron': _cronLine.text.trim(),
      'utcOffsetMinutes': (widget.utcOffset ?? DateTime.now().timeZoneOffset).inMinutes,
    } else
      'everyMs': ?_whole(_every, unit: 1000),
    'prompt': _promptText(context),
    'whenIdle': _whenIdle,
    'quietMs': ?_whole(_quiet, unit: 1000),
    'whileChildren': _whileChildren,
    'cpuBelowMs': ?_whole(_cpuBelow),
    'memoryNotShrinking': _memoryNotShrinking,
    'maxPrompts': ?_whole(_maxPrompts),
    'stopOnInput': _stopOnInput,
  });

  static String _clock(DateTime at) =>
      '${at.hour.toString().padLeft(2, '0')}:${at.minute.toString().padLeft(2, '0')}:${at.second.toString().padLeft(2, '0')}';

  static String _megabytes(int bytes) => (bytes / (1024 * 1024)).toStringAsFixed(1);

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final muted = TextStyle(color: look.muted, fontSize: 12);
    final activity = _activity;
    final watch = activity?.watch;
    final measured = [
      if (activity?.quietMs != null) context.t('watch.quiet_for', {'seconds': (activity!.quietMs! / 1000).round()}),
      if (activity?.processes != null) context.t('watch.processes', {'count': activity!.processes}),
      if (activity?.cpuMs != null) context.t('watch.cpu', {'seconds': (activity!.cpuMs! / 1000).toStringAsFixed(1)}),
      if (activity?.memoryBytes != null) context.t('watch.memory', {'megabytes': _megabytes(activity!.memoryBytes!)}),
    ];
    Widget number(Key key, TextEditingController controller, String label) => SizedBox(
      width: 120,
      child: TextField(
        key: key,
        controller: controller,
        keyboardType: TextInputType.number,
        style: const TextStyle(fontSize: 12),
        decoration: InputDecoration(isDense: true, labelText: label, border: const OutlineInputBorder()),
      ),
    );
    Widget chip(Key key, String label, bool on, ValueChanged<bool> onChanged) => FilterChip(
      key: key,
      label: Text(label, style: const TextStyle(fontSize: 12)),
      selected: on,
      visualDensity: VisualDensity.compact,
      onSelected: onChanged,
    );
    return Column(
      key: const Key('session-watch'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(children: [
          Expanded(child: Text(measured.isEmpty ? context.t('watch.not_measured') : measured.join(' · '), key: const Key('session-watch-measured'), style: muted)),
          IconButton(
            key: const Key('session-watch-refresh'),
            tooltip: context.t('watch.refresh'),
            visualDensity: VisualDensity.compact,
            iconSize: 16,
            onPressed: _busy ? null : () => unawaited(_refresh()),
            icon: const Icon(Icons.refresh),
          ),
        ]),
        if (watch != null) ...[
          const SizedBox(height: 4),
          Text(
            watch.active
                ? context.t('watch.armed', {
                    'prompt': watch.prompt,
                    'sent': watch.promptsSent,
                    'max': watch.maxPrompts == 0 ? '∞' : watch.maxPrompts,
                    'next': _clock(watch.nextCheckAt!),
                  })
                : context.t('watch.stopped', {'why': watch.stopped ?? ''}),
            key: const Key('session-watch-state'),
            style: TextStyle(fontSize: 12, color: watch.active ? look.accent : look.warning),
          ),
          if (watch.lastCheckAt != null)
            Text(
              watch.lastPrompted
                  ? context.t('watch.last_prompted', {'at': _clock(watch.lastCheckAt!)})
                  : context.t('watch.last_skipped', {'at': _clock(watch.lastCheckAt!), 'why': watch.lastWhy ?? ''}),
              key: const Key('session-watch-last'),
              style: muted,
            ),
        ],
        const SizedBox(height: 8),
        Wrap(spacing: 6, runSpacing: 6, children: [
          for (final (choice, key) in [
            (_Prompt.proceed, 'watch.prompt.continue'),
            (_Prompt.status, 'watch.prompt.status'),
            (_Prompt.custom, 'watch.prompt.custom'),
          ])
            ChoiceChip(
              key: Key('session-watch-prompt-${choice.name}'),
              label: Text(context.t(key), style: const TextStyle(fontSize: 12)),
              selected: _prompt == choice,
              visualDensity: VisualDensity.compact,
              onSelected: (_) => setState(() => _prompt = choice),
            ),
        ]),
        if (_prompt == _Prompt.custom) ...[
          const SizedBox(height: 6),
          TextField(
            key: const Key('session-watch-custom'),
            controller: _custom,
            style: const TextStyle(fontSize: 12),
            decoration: InputDecoration(isDense: true, labelText: context.t('watch.prompt.text'), border: const OutlineInputBorder()),
          ),
        ],
        const SizedBox(height: 8),
        Wrap(spacing: 6, runSpacing: 6, crossAxisAlignment: WrapCrossAlignment.center, children: [
          ChoiceChip(
            key: const Key('session-watch-interval'),
            label: Text(context.t('watch.interval'), style: const TextStyle(fontSize: 12)),
            selected: !_cron,
            visualDensity: VisualDensity.compact,
            onSelected: (_) => setState(() => _cron = false),
          ),
          ChoiceChip(
            key: const Key('session-watch-cron'),
            label: Text(context.t('watch.cron'), style: const TextStyle(fontSize: 12)),
            selected: _cron,
            visualDensity: VisualDensity.compact,
            onSelected: (_) => setState(() => _cron = true),
          ),
          if (_cron)
            SizedBox(
              width: 160,
              child: TextField(
                key: const Key('session-watch-cron-line'),
                controller: _cronLine,
                style: const TextStyle(fontSize: 12),
                decoration: InputDecoration(isDense: true, labelText: context.t('watch.cron_line'), border: const OutlineInputBorder()),
              ),
            )
          else
            number(const Key('session-watch-every'), _every, context.t('watch.every_seconds')),
        ]),
        const SizedBox(height: 8),
        Wrap(spacing: 6, runSpacing: 6, children: [
          chip(const Key('session-watch-when-idle'), context.t('watch.when_idle'), _whenIdle, (on) => setState(() => _whenIdle = on)),
          chip(const Key('session-watch-children'), context.t('watch.while_children'), _whileChildren, (on) => setState(() => _whileChildren = on)),
          chip(const Key('session-watch-memory'), context.t('watch.memory_not_shrinking'), _memoryNotShrinking, (on) => setState(() => _memoryNotShrinking = on)),
          chip(const Key('session-watch-stop-on-input'), context.t('watch.stop_on_input'), _stopOnInput, (on) => setState(() => _stopOnInput = on)),
        ]),
        const SizedBox(height: 8),
        Wrap(spacing: 6, runSpacing: 6, children: [
          number(const Key('session-watch-quiet'), _quiet, context.t('watch.quiet_seconds')),
          number(const Key('session-watch-cpu'), _cpuBelow, context.t('watch.cpu_below_ms')),
          number(const Key('session-watch-max'), _maxPrompts, context.t('watch.max_prompts')),
        ]),
        if (_problem != null) ...[
          const SizedBox(height: 6),
          Text(_problem!, key: const Key('session-watch-problem'), style: TextStyle(fontSize: 12, color: look.warning)),
        ],
        const SizedBox(height: 8),
        Row(mainAxisAlignment: MainAxisAlignment.end, children: [
          if (watch != null)
            TextButton(
              key: const Key('session-watch-off'),
              onPressed: _busy ? null : () => unawaited(_configure({'enabled': false})),
              child: Text(context.t('watch.off'), style: const TextStyle(fontSize: 12)),
            ),
          const SizedBox(width: 6),
          FilledButton.tonal(
            key: const Key('session-watch-arm'),
            onPressed: _busy ? null : () => unawaited(_arm(context)),
            child: Text(context.t(watch == null ? 'watch.arm' : 'watch.update'), style: const TextStyle(fontSize: 12)),
          ),
        ]),
      ],
    );
  }
}
