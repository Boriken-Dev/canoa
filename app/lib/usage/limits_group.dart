import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../settings/settings_row.dart';
import '../shell/confirm_remove.dart';
import 'limit_rules.dart';

/// The Usage section's Limits group: each limit of the host with how much of it is used, its state and when its window resets, and
/// the means to add, change and remove one. [accounts] and [profiles] are the names the scope pickers offer.
class LimitsGroup extends StatefulWidget {
  const LimitsGroup({super.key, required this.controller, this.accounts = const [], this.profiles = const [], this.clock});

  final LimitsController controller;
  final List<String> accounts;
  final List<String> profiles;

  /// The time as Unix seconds; the system's by default.
  final int Function()? clock;

  @override
  State<LimitsGroup> createState() => _LimitsGroupState();
}

class _LimitsGroupState extends State<LimitsGroup> {
  String? _problem, _told;

  LimitsController get _limits => widget.controller;

  @override
  void initState() {
    super.initState();
    unawaited(_limits.refresh());
  }

  Future<void> _edit({String? name}) async {
    await showDialog<void>(
      context: context,
      builder: (context) => LimitForm(controller: _limits, name: name, accounts: widget.accounts, profiles: widget.profiles),
    );
  }

  Future<void> _remove(String name) async {
    if (!await confirmRemoval(
      context,
      title: context.t('limits.remove_title', {'name': name}),
      body: context.t('limits.remove_body'),
      confirmLabel: context.t('limits.remove'),
      cancelLabel: context.t('limits.keep'),
    )) {
      return;
    }
    final refused = await _limits.remove(name);
    if (mounted) setState(() => _problem = refused);
  }

  Future<void> _allow(String name) async {
    final refused = await _limits.allow(name);
    if (!mounted) return;
    setState(() {
      _problem = refused;
      _told = refused == null ? context.t('limits.allowed', {'name': name}) : null;
    });
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _limits,
    builder: (context, _) {
      final scheme = Theme.of(context).colorScheme;
      final text = Theme.of(context).textTheme;
      final small = text.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
      final now = (widget.clock ?? () => DateTime.now().millisecondsSinceEpoch ~/ 1000)();
      final problem = _problem ?? _limits.problem;
      final mine = _limits.own, given = _limits.inherited;
      return SettingsGroup(
        key: const Key('limits-group'),
        title: context.t('limits.title'),
        trailing: TextButton.icon(
          key: const Key('limit-add'),
          onPressed: () => unawaited(_edit()),
          icon: const Icon(Icons.add, size: 18),
          label: Text(context.t('limits.add')),
        ),
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 12, 16, 4),
            child: Text(context.t('limits.intro'), key: const Key('limits-intro'), style: small),
          ),
          if (problem != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
              child: Text(problem, key: const Key('limits-problem'), style: small?.copyWith(color: scheme.error)),
            ),
          if (_told != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
              child: Text(_told!, key: const Key('limits-told'), style: small),
            ),
          if (!_limits.loaded)
            const Padding(padding: EdgeInsets.all(16), child: LinearProgressIndicator())
          else if (_limits.rows.isEmpty)
            Padding(
              padding: const EdgeInsets.all(16),
              child: Text(context.t('limits.empty'), key: const Key('limits-empty'), style: small),
            ),
          for (final row in _limits.rows)
            LimitTile(
              row: row,
              now: now,
              editable: mine.containsKey(row.name) && !given.contains(row.name),
              onEdit: () => unawaited(_edit(name: row.name)),
              onRemove: () => unawaited(_remove(row.name)),
              onAllow: row.state == LimitState.stop ? () => unawaited(_allow(row.name)) : null,
            ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
            child: Text(context.t('limits.spend_before'), key: const Key('limits-before'), style: small),
          ),
        ],
      );
    },
  );
}

/// How a limit's figure reads: `12.50 of 25.00 USD`, `600 of 1000 tokens`, or just the share for an allowance.
String limitFigureText(BuildContext context, LimitRow row) {
  final (used, limit) = (row.used, row.limit);
  if (used == null || limit == null) return '';
  return switch (row.kind) {
    LimitKind.usd => context.t('limits.used_of_usd', {'used': _dollars(used), 'limit': _dollars(limit)}),
    LimitKind.tokens => context.t('limits.used_of_tokens', {'used': used, 'limit': limit}),
    LimitKind.bucket => '',
  };
}

/// A share in thousandths as a person reads it: `80` for 800, `82.5` for 825.
String limitPercent(int permille) => permille % 10 == 0 ? '${permille ~/ 10}' : (permille / 10).toStringAsFixed(1);

String _dollars(int micro) => (micro / 1000000).toStringAsFixed(2);

/// A Unix second as a UTC date and time, `2026-11-30 00:00`.
String limitTime(int seconds) {
  final at = DateTime.fromMillisecondsSinceEpoch(seconds * 1000, isUtc: true);
  String two(int n) => n.toString().padLeft(2, '0');
  return '${at.year}-${two(at.month)}-${two(at.day)} ${two(at.hour)}:${two(at.minute)}';
}

/// The reason a figure is not measured, in the words of the app's language when the host's is one it knows.
String limitWhy(BuildContext context, String why) => switch (why) {
  'this account reports no cost' => context.t('limits.why.no_cost'),
  'nothing spent here reports a cost' => context.t('limits.why.no_cost_here'),
  'reset, not yet reported' => context.t('limits.why.reset'),
  _ => why,
};

/// One limit: its name and state, whose use and what it counts, a bar of how much is used, when it resets and where it warns and stops.
class LimitTile extends StatelessWidget {
  const LimitTile({super.key, required this.row, required this.now, this.editable = false, this.onEdit, this.onRemove, this.onAllow});

  final LimitRow row;
  final int now;
  final bool editable;
  final VoidCallback? onEdit, onRemove, onAllow;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final small = text.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final colour = switch (row.state) {
      LimitState.stop => scheme.error,
      LimitState.warn => scheme.tertiary,
      LimitState.ok => scheme.primary,
      LimitState.unmeasured => scheme.outline,
    };
    final scope = switch (row.scope) {
      LimitScope.all => context.t('limits.scope.all'),
      LimitScope.account => context.t('limits.scope.account', {'name': row.scopeName}),
      LimitScope.profile => context.t('limits.scope.profile', {'name': row.scopeName}),
    };
    final window = row.windowStart == null || row.resetsAt == null ? null : _windowOf(row);
    final figure = switch (row.kind) {
      LimitKind.bucket => context.t(row.bucket == null ? 'limits.figure.bucket_any' : 'limits.figure.bucket', {'bucket': row.bucket}),
      LimitKind.usd => window == null ? context.t('limits.figure.usd_plain') : context.t('limits.figure.usd', {'window': context.t('limits.window.$window')}),
      LimitKind.tokens => window == null ? context.t('limits.figure.tokens_plain') : context.t('limits.figure.tokens', {'window': context.t('limits.window.$window')}),
    };
    final share = row.permille;
    final stop = row.stopPermille;
    return Padding(
      key: Key('limit-row ${row.name}'),
      padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Row(children: [
          Expanded(child: Text(row.name, style: text.bodyLarge)),
          Padding(
            padding: const EdgeInsets.only(right: 8),
            child: Text(context.t('limits.state.${row.state.name}'), key: Key('limit-state ${row.name}'), style: text.bodyMedium?.copyWith(color: colour)),
          ),
        ]),
        if (onAllow != null || editable)
          Wrap(crossAxisAlignment: WrapCrossAlignment.center, children: [
            if (onAllow != null)
              TextButton(key: Key('limit-allow ${row.name}'), onPressed: onAllow, child: Text(context.t('limits.allow'))),
            if (editable) ...[
              IconButton(key: Key('limit-edit ${row.name}'), tooltip: context.t('limits.edit'), icon: const Icon(Icons.edit_outlined, size: 18), onPressed: onEdit),
              IconButton(key: Key('limit-remove ${row.name}'), tooltip: context.t('limits.remove'), icon: const Icon(Icons.delete_outline, size: 18), onPressed: onRemove),
            ],
          ]),
        Text('$scope · $figure', key: Key('limit-what ${row.name}'), style: small),
        if (share != null)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 6),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(3),
              child: LinearProgressIndicator(
                key: Key('limit-bar ${row.name}'),
                value: (share / 1000).clamp(0.0, 1.0),
                minHeight: 6,
                color: colour,
                backgroundColor: scheme.surfaceContainerHighest,
              ),
            ),
          ),
        if (share != null)
          Text(
            [context.t('limits.percent', {'percent': limitPercent(share)}), limitFigureText(context, row)].where((part) => part.isNotEmpty).join(' · '),
            key: Key('limit-figure ${row.name}'),
            style: small,
          ),
        if (row.resetsAt case final at?)
          Text(context.t('limits.resets', {'when': limitTime(at)}), key: Key('limit-resets ${row.name}'), style: small),
        Text(
          context.t('limits.thresholds', {
            'warn': limitPercent(row.warnPermille),
            'stop': stop == null ? context.t('limits.never_stops') : context.t('limits.stops_from', {'stop': limitPercent(stop)}),
          }),
          key: Key('limit-thresholds ${row.name}'),
          style: small,
        ),
        if (row.state == LimitState.unmeasured && row.why != null)
          Text(context.t('limits.not_measured', {'why': limitWhy(context, row.why!)}), key: Key('limit-why ${row.name}'), style: small?.copyWith(color: scheme.tertiary)),
      ]),
    );
  }

  /// `day`, `week` or `month`: which of the windows a money or token limit's span is.
  static String? _windowOf(LimitRow row) {
    final days = (row.resetsAt! - row.windowStart!) / 86400;
    return days <= 1 ? 'day' : days <= 7 ? 'week' : 'month';
  }
}

/// The form of one limit: its name (for a new one), whose use it counts, what it counts and where it warns and stops.
class LimitForm extends StatefulWidget {
  const LimitForm({super.key, required this.controller, this.name, this.accounts = const [], this.profiles = const []});

  final LimitsController controller;

  /// The limit being changed; null adds one.
  final String? name;
  final List<String> accounts;
  final List<String> profiles;

  @override
  State<LimitForm> createState() => _LimitFormState();
}

class _LimitFormState extends State<LimitForm> {
  final _name = TextEditingController(), _scopeName = TextEditingController(), _figure = TextEditingController();
  final _warn = TextEditingController(), _stop = TextEditingController();
  LimitScope _scope = LimitScope.all;
  LimitKind _kind = LimitKind.usd;
  String _window = 'week';
  String? _problem;
  bool _saving = false;

  @override
  void initState() {
    super.initState();
    final held = widget.name == null ? null : widget.controller.own[widget.name];
    if (held != null) {
      final read = readEntry(held);
      _scope = read.scope;
      _scopeName.text = read.scopeName;
      _kind = read.kind;
      _figure.text = read.figure;
      _window = read.window;
      _warn.text = read.warn == null ? '' : '${read.warn}';
      _stop.text = read.stop == null ? '' : '${read.stop}';
    }
  }

  @override
  void dispose() {
    for (final each in [_name, _scopeName, _figure, _warn, _stop]) {
      each.dispose();
    }
    super.dispose();
  }

  static final _validName = RegExp(r'^[a-z0-9_-]{1,32}$');

  /// A threshold typed in thousandths: empty is none, else a whole number. False for anything else.
  (bool, int?) _threshold(String text) {
    if (text.trim().isEmpty) return (true, null);
    final value = int.tryParse(text.trim());
    return (value != null && value >= 0, value);
  }

  Future<void> _save() async {
    final adding = widget.name == null;
    final name = adding ? _name.text.trim() : widget.name!;
    String? problem;
    if (!_validName.hasMatch(name)) {
      problem = context.t('limits.form.name_needed');
    } else if (adding && (widget.controller.rows.any((row) => row.name == name) || widget.controller.own.containsKey(name))) {
      problem = context.t('limits.form.name_taken', {'name': name});
    }
    final figure = _figure.text.trim();
    if (problem == null && _kind == LimitKind.bucket && figure.isEmpty) problem = context.t('limits.form.bucket_needed');
    if (problem == null && _kind != LimitKind.bucket) {
      final amount = num.tryParse(figure);
      if (amount == null || !amount.isFinite || amount <= 0 || (_kind == LimitKind.tokens && amount != amount.truncate())) {
        problem = context.t('limits.form.amount_needed');
      }
    }
    final (warnOk, warn) = _threshold(_warn.text);
    final (stopOk, stop) = _threshold(_stop.text);
    if (problem == null && (!warnOk || !stopOk || (stop != null && warn != null && stop < warn))) {
      problem = context.t('limits.form.threshold');
    }
    if (problem != null) {
      setState(() => _problem = problem);
      return;
    }
    setState(() {
      _problem = null;
      _saving = true;
    });
    final refused = await widget.controller.save(
      name,
      limitEntry(scope: _scope, scopeName: _scopeName.text.trim(), kind: _kind, figure: figure, window: _window, warn: warn, stop: stop),
    );
    if (!mounted) return;
    if (refused == null) {
      Navigator.pop(context);
    } else {
      setState(() {
        _problem = refused;
        _saving = false;
      });
    }
  }

  Widget _drop<T>(Key key, String label, T value, List<(T, String)> choices, ValueChanged<T> onChanged) => InputDecorator(
    decoration: InputDecoration(labelText: label, isDense: true, border: const OutlineInputBorder()),
    child: DropdownButtonHideUnderline(
      child: DropdownButton<T>(
        key: key,
        value: value,
        isExpanded: true,
        isDense: true,
        items: [for (final (choice, words) in choices) DropdownMenuItem<T>(value: choice, child: Text(words, overflow: TextOverflow.ellipsis))],
        onChanged: (chosen) {
          if (chosen != null) onChanged(chosen);
        },
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final names = switch (_scope) {
      LimitScope.account => widget.accounts,
      LimitScope.profile => widget.profiles,
      LimitScope.all => const <String>[],
    };
    final gap = const SizedBox(height: 12);
    return AlertDialog(
      key: const Key('limit-form'),
      title: Text(widget.name == null ? context.t('limits.form.add_title') : context.t('limits.form.edit_title', {'name': widget.name})),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            if (widget.name == null) ...[
              TextField(
                key: const Key('limit-name'),
                controller: _name,
                decoration: InputDecoration(labelText: context.t('limits.form.name'), hintText: context.t('limits.form.name_hint')),
              ),
              gap,
            ],
            _drop<LimitScope>(
              const Key('limit-scope'),
              context.t('limits.form.scope'),
              _scope,
              [
                (LimitScope.all, context.t('limits.scope.all')),
                (LimitScope.account, context.t('limits.form.account')),
                (LimitScope.profile, context.t('limits.form.profile')),
              ],
              (chosen) => setState(() {
                _scope = chosen;
                _scopeName.clear();
                if (chosen == LimitScope.profile && _kind == LimitKind.bucket) _kind = LimitKind.usd;
              }),
            ),
            if (_scope != LimitScope.all) ...[
              gap,
              if (names.isNotEmpty)
                _drop<String>(
                  const Key('limit-scope-name'),
                  context.t(_scope == LimitScope.account ? 'limits.form.account' : 'limits.form.profile'),
                  names.contains(_scopeName.text) ? _scopeName.text : names.first,
                  [for (final each in names) (each, each)],
                  (chosen) => setState(() => _scopeName.text = chosen),
                )
              else
                TextField(
                  key: const Key('limit-scope-name'),
                  controller: _scopeName,
                  decoration: InputDecoration(labelText: context.t(_scope == LimitScope.account ? 'limits.form.account' : 'limits.form.profile')),
                ),
            ],
            gap,
            _drop<LimitKind>(
              const Key('limit-kind'),
              context.t('limits.form.what'),
              _kind,
              [
                if (_scope != LimitScope.profile) (LimitKind.bucket, context.t('limits.form.bucket')),
                (LimitKind.usd, context.t('limits.form.usd')),
                (LimitKind.tokens, context.t('limits.form.tokens')),
              ],
              (chosen) => setState(() => _kind = chosen),
            ),
            gap,
            TextField(
              key: const Key('limit-figure'),
              controller: _figure,
              keyboardType: _kind == LimitKind.bucket ? TextInputType.text : const TextInputType.numberWithOptions(decimal: true),
              decoration: InputDecoration(
                labelText: context.t(switch (_kind) {
                  LimitKind.bucket => 'limits.form.bucket_id',
                  LimitKind.usd => 'limits.form.amount_usd',
                  LimitKind.tokens => 'limits.form.amount_tokens',
                }),
                hintText: _kind == LimitKind.bucket ? context.t('limits.form.bucket_hint') : null,
              ),
            ),
            if (_kind != LimitKind.bucket) ...[
              gap,
              _drop<String>(
                const Key('limit-window'),
                context.t('limits.form.window'),
                _window,
                [for (final each in const ['day', 'week', 'month']) (each, context.t('limits.window.$each'))],
                (chosen) => setState(() => _window = chosen),
              ),
            ],
            gap,
            TextField(
              key: const Key('limit-warn'),
              controller: _warn,
              keyboardType: TextInputType.number,
              decoration: InputDecoration(labelText: context.t('limits.form.warn'), hintText: context.t('limits.form.warn_hint')),
            ),
            gap,
            TextField(
              key: const Key('limit-stop'),
              controller: _stop,
              keyboardType: TextInputType.number,
              decoration: InputDecoration(labelText: context.t('limits.form.stop'), hintText: context.t('limits.form.stop_hint')),
            ),
            if (_problem != null)
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: Text(_problem!, key: const Key('limit-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
              ),
          ]),
        ),
      ),
      actions: [
        TextButton(key: const Key('limit-cancel'), onPressed: _saving ? null : () => Navigator.pop(context), child: Text(context.t('limits.cancel'))),
        FilledButton(key: const Key('limit-save'), onPressed: _saving ? null : () => unawaited(_save()), child: Text(context.t('limits.save'))),
      ],
    );
  }
}

/// What a refused turn says of the limit that refused it: the `_meta` the host's answer carries under `dev.boriken.canoa/limit`.
final class LimitStop {
  const LimitStop({required this.limit, required this.usedPermille, this.resetsAt});

  /// The `_meta` key of a refused turn's answer.
  static const metaKey = 'dev.boriken.canoa/limit';

  /// The stop the limit [row] puts on a session, or null when it is not stopping.
  static LimitStop? ofRow(LimitRow row) =>
      row.state == LimitState.stop ? LimitStop(limit: row.name, usedPermille: row.permille ?? 0, resetsAt: row.resetsAt) : null;

  /// The stop a prompt's answer names, or null when it names none.
  static LimitStop? of(Object? result) {
    final meta = result is Map ? result['_meta'] : null;
    final held = meta is Map ? meta[metaKey] : null;
    if (held is! Map || held['limit'] is! String) return null;
    final used = held['used_permille'], resets = held['resets_at'];
    return LimitStop(limit: held['limit'] as String, usedPermille: used is num ? used.toInt() : 0, resetsAt: resets is num ? resets.toInt() : null);
  }

  final String limit;
  final int usedPermille;
  final int? resetsAt;
}

/// The card of a turn a limit refused: which limit, how much is used, when it resets, and the two things to do about it.
class LimitStopCard extends StatelessWidget {
  const LimitStopCard({super.key, required this.stop, this.onContinue, this.onOpenLimits});

  final LimitStop stop;

  /// Lets one turn through the limit; the card offers it when given.
  final VoidCallback? onContinue;
  final VoidCallback? onOpenLimits;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    return Card(
      key: Key('limit-stop-card ${stop.limit}'),
      color: scheme.errorContainer,
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Row(children: [
            Icon(Icons.block, color: scheme.onErrorContainer),
            const SizedBox(width: 8),
            Expanded(child: Text(context.t('limits.stop.title', {'name': stop.limit}), style: text.titleSmall?.copyWith(color: scheme.onErrorContainer))),
          ]),
          const SizedBox(height: 8),
          Text(
            [
              context.t('limits.stop.figure', {'percent': limitPercent(stop.usedPermille)}),
              if (stop.resetsAt case final at?) context.t('limits.resets', {'when': limitTime(at)}),
            ].join(' · '),
            key: const Key('limit-stop-figure'),
            style: text.bodyMedium?.copyWith(color: scheme.onErrorContainer),
          ),
          const SizedBox(height: 12),
          Wrap(spacing: 8, runSpacing: 8, children: [
            if (onContinue != null) FilledButton(key: const Key('limit-stop-continue'), onPressed: onContinue, child: Text(context.t('limits.stop.continue'))),
            if (onOpenLimits != null) OutlinedButton(key: const Key('limit-stop-open'), onPressed: onOpenLimits, child: Text(context.t('limits.stop.open'))),
          ]),
        ]),
      ),
    );
  }
}
