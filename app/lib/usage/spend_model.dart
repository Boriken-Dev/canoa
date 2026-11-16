/// What a host summed for one group of its usage records (or for all of them): a counter is null when no record in the group
/// reported it, which is not the same as zero.
final class SpendRow {
  const SpendRow({
    this.key,
    this.records = 0,
    this.sessions = 0,
    this.input,
    this.output,
    this.cacheRead,
    this.cacheWrite,
    this.costMicroUsd,
  });

  /// A `usage/summary` group or total; null when [json] is not one.
  static SpendRow? parse(Object? json) {
    if (json is! Map) return null;
    int? whole(String name) {
      final value = json[name];
      return value is int && value >= 0 ? value : null;
    }

    final key = json['key'];
    return SpendRow(
      key: key is String ? key : null,
      records: whole('records') ?? 0,
      sessions: whole('sessions') ?? 0,
      input: whole('input'),
      output: whole('output'),
      cacheRead: whole('cache_read'),
      cacheWrite: whole('cache_write'),
      costMicroUsd: whole('cost_micro_usd'),
    );
  }

  /// The group's name: a day, an account's identity, a harness, a model or a session; null where the records name none.
  final String? key;
  final int records, sessions;

  /// Tokens the model read that were not served from the cache.
  final int? input;
  final int? output, cacheRead, cacheWrite;

  /// Cost the agents reported, in millionths of a US dollar.
  final int? costMicroUsd;

  /// All four token counters, or null when none was reported.
  int? get tokens {
    final counters = [input, output, cacheRead, cacheWrite].nonNulls;
    return counters.isEmpty ? null : counters.fold<int>(0, (a, b) => a + b);
  }

  /// The two rows added; a counter only one reported keeps its value, and one neither reported stays absent.
  SpendRow plus(SpendRow other, {String? key}) {
    int? add(int? a, int? b) => a == null && b == null ? null : (a ?? 0) + (b ?? 0);
    return SpendRow(
      key: key,
      records: records + other.records,
      sessions: sessions + other.sessions,
      input: add(input, other.input),
      output: add(output, other.output),
      cacheRead: add(cacheRead, other.cacheRead),
      cacheWrite: add(cacheWrite, other.cacheWrite),
      costMicroUsd: add(costMicroUsd, other.costMicroUsd),
    );
  }
}

/// A host's answer to `usage/summary`, or several hosts' answers added.
final class SpendSummary {
  const SpendSummary({this.timezone = 'UTC', this.total = const SpendRow(), this.groups = const []});

  static SpendSummary? parse(Object? json) {
    if (json is! Map) return null;
    final groups = json['groups'];
    final zone = json['timezone'];
    return SpendSummary(
      timezone: zone is String ? zone : 'UTC',
      total: SpendRow.parse(json['total']) ?? const SpendRow(),
      groups: [if (groups is List) for (final each in groups) ?SpendRow.parse(each)],
    );
  }

  /// The groups of every summary, one for each key; the totals added.
  static SpendSummary merge(List<SpendSummary> all) {
    if (all.length == 1) return all.first;
    final byKey = <String?, SpendRow>{};
    var total = const SpendRow();
    for (final summary in all) {
      total = total.plus(summary.total);
      for (final group in summary.groups) {
        final seen = byKey[group.key];
        byKey[group.key] = seen == null ? group : seen.plus(group, key: group.key);
      }
    }
    return SpendSummary(timezone: all.isEmpty ? 'UTC' : all.first.timezone, total: total, groups: byKey.values.toList());
  }

  /// The zone the days are cut in.
  final String timezone;
  final SpendRow total;
  final List<SpendRow> groups;
}

/// The Tokens and Cost views' ranges.
enum SpendRange {
  day('Past 24 hours', 1),
  week('7 days', 7),
  month('30 days', 30),
  quarter('90 days', 90),
  custom('Custom', 0);

  const SpendRange(this.label, this.days);

  final String label;
  final int days;
}

/// What the tables break the spend down by.
enum Breakdown {
  account('Account'),
  provider('Provider'),
  model('Model'),
  host('Host'),
  session('Session'),
  day('Day');

  const Breakdown(this.label);

  final String label;
}

/// Which of the Usage section's views is shown.
enum UsageView {
  limits('Limits'),
  tokens('Tokens'),
  cost('Cost');

  const UsageView(this.label);

  final String label;
}
