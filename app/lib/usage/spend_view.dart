import 'dart:async';

import 'package:flutter/material.dart';

import 'chart.dart';
import 'spend_model.dart';
import 'usage_model.dart';

/// Said where a counter was not reported, which is not the same as none used.
const notReported = 'Not reported';

/// One figure a view sums: a token counter, or the cost.
final class MetricColumn {
  const MetricColumn(this.label, this.help, this.of, this.format);

  final String label;

  /// What the figure means, said in a tooltip.
  final String help;
  final int? Function(SpendRow row) of;
  final String Function(int value) format;

  String text(SpendRow row) {
    final value = of(row);
    return value == null ? notReported : format(value);
  }
}

/// What a Tokens or a Cost view is made of: its figures, and the one the chart and the account shares are drawn in.
final class SpendMetric {
  const SpendMetric({required this.name, required this.columns, required this.chart, required this.chartFormat, this.note});

  final String name;
  final List<MetricColumn> columns;

  /// The figure of a row the chart and the account shares use, null when the row reported none.
  final int? Function(SpendRow row) chart;
  final String Function(double value) chartFormat;

  /// Said under the totals.
  final String? note;
}

/// 1,234,567.
String groupDigits(int value) {
  final text = '$value';
  final out = StringBuffer();
  for (var i = 0; i < text.length; i++) {
    if (i > 0 && (text.length - i) % 3 == 0) out.write(',');
    out.write(text[i]);
  }
  return out.toString();
}

String _two(int value) => value.toString().padLeft(2, '0');

/// A day as the hosts name it: `2026-11-16`, from Unix seconds.
String dayKey(int seconds) {
  final date = DateTime.fromMillisecondsSinceEpoch(seconds * 1000, isUtc: true);
  return '${date.year.toString().padLeft(4, '0')}-${_two(date.month)}-${_two(date.day)}';
}

/// The Tokens and Cost views' shared page: range, totals, each account's part, a daily chart with its table, and the breakdowns.
class SpendView extends StatelessWidget {
  const SpendView({super.key, required this.controller, required this.metric});

  final UsageController controller;
  final SpendMetric metric;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final small = text.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final total = controller.total;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _RangeBar(controller: controller),
        if (controller.model != null || controller.session != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Wrap(
              spacing: 8,
              children: [
                if (controller.model != null)
                  InputChip(
                    key: const Key('spend-chip model'),
                    label: Text('Model: ${controller.model}'),
                    onDeleted: () => controller.selectModel(null),
                  ),
                if (controller.session != null)
                  InputChip(
                    key: const Key('spend-chip session'),
                    label: Text('Session: ${controller.session}'),
                    onDeleted: () => controller.selectSession(null),
                  ),
              ],
            ),
          ),
        for (final MapEntry(:key, :value) in controller.notCovered.entries)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              '$key is not covered: it did not answer ($value)',
              key: Key('spend-uncovered $key'),
              style: small?.copyWith(color: scheme.error),
            ),
          ),
        const SizedBox(height: 16),
        Wrap(
          spacing: 12,
          runSpacing: 12,
          children: [
            _Tile(label: 'Sessions', value: groupDigits(total.sessions), tileKey: const Key('spend-total Sessions')),
            for (final column in metric.columns)
              _Tile(label: column.label, help: column.help, value: column.text(total), tileKey: Key('spend-total ${column.label}')),
          ],
        ),
        if (metric.note != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(metric.note!, key: const Key('spend-note'), style: small),
          ),
        const SizedBox(height: 24),
        Text('By account', style: text.titleSmall),
        const SizedBox(height: 8),
        _Contributions(controller: controller, metric: metric),
        const SizedBox(height: 24),
        Text('By day', style: text.titleSmall),
        const SizedBox(height: 8),
        _Daily(controller: controller, metric: metric),
        const SizedBox(height: 24),
        Text('Breakdown', style: text.titleSmall),
        const SizedBox(height: 8),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            for (final each in Breakdown.values)
              ChoiceChip(
                key: Key('spend-breakdown ${each.name}'),
                label: Text(each.label),
                selected: controller.breakdown == each,
                onSelected: (_) => controller.setBreakdown(each),
              ),
          ],
        ),
        const SizedBox(height: 8),
        _BreakdownTable(key: ValueKey('spend-table ${controller.breakdown.name}'), controller: controller, metric: metric),
        if (controller.spendLoading)
          const Padding(padding: EdgeInsets.only(top: 12), child: LinearProgressIndicator(key: Key('spend-loading'))),
      ],
    );
  }
}

class _Tile extends StatelessWidget {
  const _Tile({required this.label, required this.value, required this.tileKey, this.help});

  final String label, value;
  final String? help;
  final Key tileKey;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final scheme = Theme.of(context).colorScheme;
    final caption = Text(label, style: text.labelMedium?.copyWith(color: scheme.onSurfaceVariant));
    return Container(
      key: tileKey,
      constraints: const BoxConstraints(minWidth: 140),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(border: Border.all(color: scheme.outlineVariant), borderRadius: BorderRadius.circular(8)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          help == null ? caption : Tooltip(message: help!, child: caption),
          const SizedBox(height: 4),
          Text(value, style: value == notReported ? text.bodyMedium?.copyWith(color: scheme.onSurfaceVariant) : text.titleLarge),
        ],
      ),
    );
  }
}

class _RangeBar extends StatelessWidget {
  const _RangeBar({required this.controller});

  final UsageController controller;

  Future<void> _custom(BuildContext context) async {
    final now = DateTime.fromMillisecondsSinceEpoch(controller.clock * 1000, isUtc: true);
    final picked = await showDateRangePicker(
      context: context,
      firstDate: DateTime.utc(now.year - 2),
      lastDate: DateTime.utc(now.year, now.month, now.day),
      initialDateRange: DateTimeRange(start: now.subtract(const Duration(days: 7)), end: now),
    );
    if (picked == null) return;
    int day(DateTime date) => DateTime.utc(date.year, date.month, date.day).millisecondsSinceEpoch ~/ 1000;
    controller.setCustom(day(picked.start), day(picked.end));
  }

  @override
  Widget build(BuildContext context) {
    final small = Theme.of(context).textTheme.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant);
    final (from, to) = controller.span;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            for (final each in SpendRange.values)
              ChoiceChip(
                key: Key('spend-range ${each.name}'),
                label: Text(each.label),
                selected: controller.range == each,
                onSelected: (_) => each == SpendRange.custom ? unawaited(_custom(context)) : controller.setRange(each),
              ),
          ],
        ),
        const SizedBox(height: 6),
        Text(
          '${dayKey(from)} to ${dayKey(to - 1)} · days are in ${controller.timezone}',
          key: const Key('spend-zone'),
          style: small,
        ),
      ],
    );
  }
}

class _Contributions extends StatelessWidget {
  const _Contributions({required this.controller, required this.metric});

  final UsageController controller;
  final SpendMetric metric;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final small = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final groups = controller.byAccount.groups;
    if (groups.isEmpty) return Text('Nothing recorded in this range.', key: const Key('spend-empty'), style: small);
    final sorted = [...groups]..sort((a, b) => (metric.chart(b) ?? -1).compareTo(metric.chart(a) ?? -1));
    final largest = sorted.map((each) => metric.chart(each) ?? 0).fold<int>(0, (a, b) => b > a ? b : a);
    final column = metric.columns.length == 1 ? metric.columns.first : null;
    return Column(
      children: [
        for (final row in sorted)
          Padding(
            key: Key('spend-account ${row.key ?? ''}'),
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: Row(
              children: [
                Expanded(flex: 3, child: Text(controller.accountName(row.key), overflow: TextOverflow.ellipsis)),
                Expanded(
                  flex: 4,
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(3),
                    child: LinearProgressIndicator(
                      value: largest == 0 ? 0 : (metric.chart(row) ?? 0) / largest,
                      minHeight: 8,
                      backgroundColor: scheme.surfaceContainerHighest,
                    ),
                  ),
                ),
                const SizedBox(width: 12),
                SizedBox(
                  width: 110,
                  child: Text(
                    column != null ? column.text(row) : _tokensText(metric, row),
                    textAlign: TextAlign.end,
                    style: row.tokens == null && column == null ? small : null,
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

String _tokensText(SpendMetric metric, SpendRow row) {
  final value = metric.chart(row);
  return value == null ? notReported : groupDigits(value);
}

class _Daily extends StatelessWidget {
  const _Daily({required this.controller, required this.metric});

  final UsageController controller;
  final SpendMetric metric;

  List<String> _days() {
    final (from, to) = controller.span;
    final days = <String>[];
    for (var at = from - from % 86400; at < to && days.length < 400; at += 86400) {
      days.add(dayKey(at));
    }
    return days;
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final small = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final byKey = {for (final row in controller.byDay.groups) row.key: row};
    final bars = [
      for (final day in _days()) ChartBar(day, byKey[day] == null ? null : metric.chart(byKey[day]!)?.toDouble()),
    ];
    final chart = UsageBarChart(
      bars: bars,
      format: metric.chartFormat,
      description: '${metric.name} by day, ${bars.length} days',
    );
    final rows = [...controller.byDay.groups]..sort((a, b) => (a.key ?? '').compareTo(b.key ?? ''));
    final table = rows.isEmpty
        ? Text('Nothing recorded in this range.', key: const Key('spend-day-empty'), style: small)
        : ConstrainedBox(
            key: const Key('spend-day-table'),
            constraints: const BoxConstraints(maxHeight: 280),
            child: SingleChildScrollView(
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: DataTable(
                  columnSpacing: 20,
                  headingRowHeight: 36,
                  dataRowMinHeight: 32,
                  dataRowMaxHeight: 36,
                  columns: [
                    const DataColumn(label: Text('Day')),
                    for (final column in metric.columns) DataColumn(label: _Head(column), numeric: true),
                  ],
                  rows: [
                    for (final row in rows)
                      DataRow(
                        key: ValueKey('spend-day ${row.key}'),
                        cells: [
                          DataCell(Text(row.key ?? 'Unknown', key: Key('spend-day ${row.key}'))),
                          for (final column in metric.columns) DataCell(Text(column.text(row), key: Key('spend-day ${row.key} ${column.label}'))),
                        ],
                      ),
                  ],
                ),
              ),
            ),
          );
    return LayoutBuilder(
      // Side by side only where the table fits whole beside a chart; a table of many columns goes under it.
      builder: (context, box) => box.maxWidth >= 420 + 130 * metric.columns.length
          ? Row(
              key: const Key('spend-daily wide'),
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Expanded(child: chart),
                const SizedBox(width: 16),
                Expanded(child: table),
              ],
            )
          : Column(
              key: const Key('spend-daily narrow'),
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [chart, const SizedBox(height: 12), table],
            ),
    );
  }
}

class _Head extends StatelessWidget {
  const _Head(this.column);

  final MetricColumn column;

  @override
  Widget build(BuildContext context) => Tooltip(message: column.help, child: Text(column.label));
}

class _BreakdownTable extends StatefulWidget {
  const _BreakdownTable({super.key, required this.controller, required this.metric});

  final UsageController controller;
  final SpendMetric metric;

  @override
  State<_BreakdownTable> createState() => _BreakdownTableState();
}

class _BreakdownTableState extends State<_BreakdownTable> {
  // Column 0 is the name, 1 the session count, then the metric's columns; the biggest first until the person chooses.
  int _column = 2;
  bool _ascending = false;

  static const _shown = 200;

  String _name(SpendRow row) {
    final key = row.key;
    return switch (widget.controller.breakdown) {
      Breakdown.account => widget.controller.accountName(key),
      Breakdown.model => key ?? 'Not named',
      _ => key ?? 'Unknown',
    };
  }

  num _of(SpendRow row, int column) => column == 1 ? row.sessions : (widget.metric.columns[column - 2].of(row) ?? -1);

  void _drill(SpendRow row) {
    final key = row.key;
    final usage = widget.controller;
    if (key == null || key.isEmpty) return;
    switch (usage.breakdown) {
      case Breakdown.account:
        usage.selectAccount(key);
      case Breakdown.provider:
        usage.selectProvider(key);
      case Breakdown.model:
        usage.selectModel(key);
      case Breakdown.host:
        usage.selectHost(key);
      case Breakdown.session:
        usage.selectSession(key);
      case Breakdown.day:
        final start = (DateTime.parse('${key}T00:00:00Z').millisecondsSinceEpoch) ~/ 1000;
        usage.setCustom(start, start);
    }
  }

  @override
  Widget build(BuildContext context) {
    final small = Theme.of(context).textTheme.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant);
    final all = widget.controller.rows;
    if (all.isEmpty) return Text('Nothing recorded in this range.', key: const Key('spend-breakdown-empty'), style: small);
    final rows = [...all];
    if (_column == 0) {
      rows.sort((a, b) => _name(a).toLowerCase().compareTo(_name(b).toLowerCase()));
    } else {
      rows.sort((a, b) => _of(a, _column).compareTo(_of(b, _column)));
    }
    final ordered = _ascending ? rows : rows.reversed.toList();
    final shown = ordered.take(_shown).toList();
    void sort(int column, bool ascending) => setState(() {
      _column = column;
      _ascending = ascending;
    });
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: DataTable(
            key: const Key('spend-breakdown-table'),
            sortColumnIndex: _column,
            sortAscending: _ascending,
            showCheckboxColumn: false,
            columnSpacing: 20,
            headingRowHeight: 36,
            dataRowMinHeight: 36,
            dataRowMaxHeight: 40,
            columns: [
              DataColumn(label: Text(widget.controller.breakdown.label), onSort: sort),
              DataColumn(label: const Text('Sessions'), numeric: true, onSort: sort),
              for (final column in widget.metric.columns) DataColumn(label: _Head(column), numeric: true, onSort: sort),
            ],
            rows: [
              for (final row in shown)
                DataRow(
                  key: ValueKey('spend-row ${row.key}'),
                  onSelectChanged: (_) => _drill(row),
                  cells: [
                    DataCell(ConstrainedBox(constraints: const BoxConstraints(maxWidth: 260), child: Text(_name(row), key: Key('spend-row ${row.key}'), overflow: TextOverflow.ellipsis))),
                    DataCell(Text(groupDigits(row.sessions))),
                    for (final column in widget.metric.columns) DataCell(Text(column.text(row))),
                  ],
                ),
            ],
          ),
        ),
        if (ordered.length > shown.length)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text('Showing ${shown.length} of ${ordered.length}.', key: const Key('spend-truncated'), style: small),
          ),
      ],
    );
  }
}
