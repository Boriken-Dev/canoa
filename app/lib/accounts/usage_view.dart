import 'package:coqui_acp_chat/coqui_acp_chat.dart' show UsageBucket, UsageReport;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../usage/usage_warn.dart';

/// How much of [bucket] is left, as Claude's and Codex's usage pages say it: `38% left`, to the nearest percent. Some is never said
/// as none and some used is never said as none used. Null when the bucket gives no fraction, and for a window that has ended,
/// whose figures are of the window before.
String? leftText(UsageBucket bucket, int now) {
  final used = bucket.fractionPermille;
  if (used == null || bucket.pastReset(now)) return null;
  final left = 1000 - used;
  var percent = (left + 5) ~/ 10;
  if (left > 0 && percent == 0) percent = 1;
  if (left < 1000 && percent == 100) percent = 99;
  return '$percent% left';
}

/// The models [bucket] covers, in words: the names the harness gave, else the ids it covers, else `All models`. Null when it says
/// nothing of them, so no line is shown.
String? modelsText(UsageBucket bucket) {
  if (bucket.modelNames.isNotEmpty) return bucket.modelNames.join(', ');
  return switch (bucket.models.kind) {
    'all' => 'All models',
    'only' when bucket.models.models.isNotEmpty => bucket.models.models.join(', '),
    _ => null,
  };
}

/// The buckets as the view lists them: when the harness sorts them into groups, each group once, at the place its first bucket
/// has in the report, with its windows in the report's order; otherwise one section for each bucket, in the order of [ordered].
List<(String?, List<UsageBucket>)> usageSections(UsageReport report, int now) {
  if (!report.buckets.any((bucket) => bucket.group != null)) {
    return [for (final bucket in ordered(report, now)) (null, [bucket])];
  }
  final sections = <(String?, List<UsageBucket>)>[];
  for (final bucket in report.buckets) {
    final group = bucket.group;
    final at = group == null ? -1 : sections.indexWhere((section) => section.$1 == group);
    if (at >= 0) {
      sections[at].$2.add(bucket);
    } else {
      sections.add((group, [bucket]));
    }
  }
  return sections;
}

/// Where an API key's figure came from, in words: the provider's rate-limit headers, a gateway's key information, or the tokens this
/// host recorded. Null for a figure the provider itself reports (a plan's window), which needs no source said.
String? sourceText(UsageBucket bucket) => switch (bucket.source) {
  'response_headers' => "From the provider's rate-limit headers",
  'gateway' => "From the gateway's key information",
  'recorded' => 'Counted by Canoa from the sessions on this key',
  _ => null,
};

/// When [bucket] resets, as a countdown: `Resets in 3 h 20 m`. Null for a bucket that refills some other way.
String? resetText(UsageBucket bucket, int now) {
  final left = bucket.resetsIn(now);
  if (left == null) return null;
  return left == 0 ? 'Reset, not yet reported' : 'Resets in ${spanText(left)}';
}

/// What a window's pace says, in the words a person uses: `At this pace you'll run out Friday evening`, or that the allowance
/// lasts to the reset. Null when there is no pace to read: no fixed window, nothing used yet, or a window only just begun,
/// whose first minutes say nothing of the rest.
///
/// [local] turns a Unix second into the person's own clock; the system's by default.
String? forecastText(UsageBucket bucket, int now, {DateTime Function(int seconds)? local}) {
  final refill = bucket.refill;
  final end = refill.resetsAt, length = refill.length;
  final used = bucket.fractionPermille;
  if (!refill.isFixedWindow || end == null || length == null || used == null || bucket.pastReset(now)) return null;
  if (bucket.isExhausted || used >= 1000) return null;
  final window = length.inSeconds, elapsed = now - (end - window);
  // A tenth of the window is the least that shows a pace.
  if (window <= 0 || elapsed < window ~/ 10 || elapsed > window || used <= 0) return null;
  // Time to use what is left at the rate used so far: (1000 - used) / (used / elapsed).
  final toEmpty = (1000 - used) * elapsed ~/ used;
  final empty = now + toEmpty;
  if (empty >= end) return 'On pace to last until it resets';
  final clock = local ?? (seconds) => DateTime.fromMillisecondsSinceEpoch(seconds * 1000);
  return "At this pace you'll run out ${_when(clock(empty), clock(now))}";
}

const _days = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];

String _when(DateTime at, DateTime from) {
  final today = DateTime(from.year, from.month, from.day);
  final day = DateTime(at.year, at.month, at.day);
  final apart = day.difference(today).inDays;
  final part = switch (at.hour) {
    >= 5 && < 12 => 'morning',
    >= 12 && < 17 => 'afternoon',
    >= 17 && < 21 => 'evening',
    _ => 'night',
  };
  if (apart <= 0) {
    // Within the day, the hour is more use than a part of it.
    final hour = at.hour % 12 == 0 ? 12 : at.hour % 12;
    return 'at $hour:${at.minute.toString().padLeft(2, '0')} ${at.hour < 12 ? 'AM' : 'PM'}';
  }
  if (apart == 1) return 'tomorrow $part';
  return '${_days[at.weekday - 1]} $part';
}

/// What the composer's account picker says of an account: the shortest window and the longest, each as `5 h 90% left`, so a person
/// sees both what is left now and what is left for the week. Null when no bucket gives a fraction of a fixed window.
String? windowSummary(UsageReport? report, int now) {
  if (report == null) return null;
  final windows = [
    for (final bucket in report.buckets)
      if (bucket.refill.isFixedWindow && bucket.refill.length != null && leftText(bucket, now) != null) bucket,
  ]..sort((a, b) => a.refill.length!.compareTo(b.refill.length!));
  if (windows.isEmpty) return null;
  String say(UsageBucket bucket) => '${_windowName(bucket.refill.length!)} ${leftText(bucket, now)}';
  return windows.length == 1 ? say(windows.first) : '${say(windows.first)} · ${say(windows.last)}';
}

String _windowName(Duration length) {
  if (length >= const Duration(days: 7)) return length.inDays == 7 ? 'week' : '${length.inDays} d';
  if (length >= const Duration(hours: 24)) return '${length.inDays} d';
  return '${length.inHours} h';
}

/// How old a reading may be before it is said to be out of date.
const staleAfter = Duration(minutes: 30);

/// What an account's reports say: each bucket with how much is left, a bar, when it resets and the pace. The report is what the
/// sessions on the account last heard, so a figure says when it was observed.
class AccountUsageView extends StatelessWidget {
  const AccountUsageView({super.key, required this.report, this.now, this.warnPermille, this.large = false, this.readAt});

  final UsageReport? report;

  /// Unix seconds; the clock's when null.
  final int? now;
  /// From this many thousandths used a bucket shows as running low; the threshold of [UsageWarn] when null.
  final int? warnPermille;

  /// A page's view rather than a list row's: the figure big, a word for how it stands that colour does not carry, and when it was
  /// read.
  final bool large;

  /// Unix seconds the figures were read, for a bucket that does not say; the report's own when null.
  final int? readAt;

  @override
  Widget build(BuildContext context) {
    final at = now ?? DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final scheme = Theme.of(context).colorScheme;
    final small = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final sections = report == null ? const <(String?, List<UsageBucket>)>[] : usageSections(report!, at);
    if (sections.isEmpty) {
      return Text(
        report == null ? 'No usage heard yet: it shows once a session has run on this account.' : 'Nothing reported yet',
        key: const Key('account-usage-empty'),
        style: small,
      );
    }
    final text = Theme.of(context).textTheme;
    Widget bucketOf(UsageBucket bucket, {required bool models}) => Padding(
      key: Key('account-usage ${bucket.id}'),
      padding: const EdgeInsets.only(bottom: 12),
      child: _Bucket(
        bucket: bucket,
        now: at,
        warnPermille: warnPermille ?? UsageWarn.of(context),
        large: large,
        readAt: bucket.observedAt > 0 ? bucket.observedAt : (readAt ?? report?.observedAt ?? 0),
        models: models,
      ),
    );
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final (group, buckets) in sections)
        if (group == null)
          for (final bucket in buckets) bucketOf(bucket, models: true)
        else
          Padding(
            key: Key('account-group $group'),
            padding: const EdgeInsets.only(bottom: 4),
            child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              Text(group, style: text.titleSmall),
              // A group's windows share its models: they are said once, under its name.
              if (_sharedModels(buckets) case final models?)
                Padding(
                  padding: const EdgeInsets.only(top: 2),
                  child: Text(context.t('usage.models', {'models': models}), key: Key('account-models $group'), style: small),
                ),
              const SizedBox(height: 8),
              Padding(
                padding: const EdgeInsets.only(left: 12),
                child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                  for (final bucket in buckets) bucketOf(bucket, models: _sharedModels(buckets) == null),
                ]),
              ),
            ]),
          ),
    ]);
  }
}

/// What every bucket of a group says of its models, when they all say the same; null when one differs or none says.
String? _sharedModels(List<UsageBucket> buckets) {
  final said = {for (final bucket in buckets) modelsText(bucket)};
  return said.length == 1 ? said.first : null;
}

class _Bucket extends StatelessWidget {
  const _Bucket({required this.bucket, required this.now, required this.warnPermille, required this.large, required this.readAt, required this.models});

  final UsageBucket bucket;
  final int now;
  /// From this many thousandths used a bucket shows as running low; the threshold of [UsageWarn] when null.
  final int warnPermille;
  final bool large;
  final int readAt;

  /// Whether the models the bucket covers are said under it, as they are not once for a group.
  final bool models;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final small = text.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final left = bucket.isDisabled ? context.t('usage.disabled') : leftText(bucket, now) ?? (bucket.isExhausted ? 'Limit reached' : figureOf(bucket));
    final level = levelOf(bucket, now, warnPermille: warnPermille);
    final colour = switch (level) {
      UsageLevel.danger => scheme.error,
      UsageLevel.warn => scheme.tertiary,
      UsageLevel.calm => scheme.primary,
    };
    final used = bucket.pastReset(now) ? null : bucket.fractionPermille;
    final reset = bucket.isDisabled ? null : resetText(bucket, now), pace = forecastText(bucket, now);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Row(children: [
        Expanded(child: Text(bucket.name, style: text.bodyMedium)),
        if (left != null)
          Text(left, key: Key('account-left ${bucket.id}'), style: (large ? text.titleLarge : text.bodyMedium)?.copyWith(color: colour)),
      ]),
      if (used != null)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(3),
            child: LinearProgressIndicator(value: (1000 - used) / 1000, minHeight: 6, color: colour, backgroundColor: scheme.surfaceContainerHighest),
          ),
        ),
      if (large && level != UsageLevel.calm)
        Row(key: Key('account-state ${bucket.id}'), children: [
          Icon(level == UsageLevel.danger ? Icons.error_outline : Icons.warning_amber_rounded, size: 16, color: colour),
          const SizedBox(width: 4),
          Text(bucket.isExhausted ? 'Limit reached' : level == UsageLevel.danger ? 'Almost used up' : 'Running low', style: small?.copyWith(color: colour)),
        ]),
      if (models)
        if (modelsText(bucket) case final covered?) Text(context.t('usage.models', {'models': covered}), key: Key('account-models ${bucket.id}'), style: small),
      if (reset != null) Text(reset, key: Key('account-reset ${bucket.id}'), style: small),
      if (sourceText(bucket) case final source?) Text(source, key: Key('account-source ${bucket.id}'), style: small),
      if (pace != null) Text(pace, key: Key('account-pace ${bucket.id}'), style: small?.copyWith(color: level == UsageLevel.calm ? null : colour)),
      if (large && readAt > 0) _read(small, scheme),
    ]);
  }

  /// When the figure was read, and a warning in words once it is old enough to have changed.
  Widget _read(TextStyle? small, ColorScheme scheme) {
    final age = now - readAt;
    final stale = age > staleAfter.inSeconds;
    return Text(
      stale ? 'Last read ${agoText(age)}; it may have changed since' : 'Read ${agoText(age)}',
      key: Key('account-read ${bucket.id}'),
      style: small?.copyWith(color: stale ? scheme.tertiary : null),
    );
  }
}
