import 'package:coqui_acp_chat/coqui_acp_chat.dart' show UsageBucket, UsageReport;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

/// How much of [bucket] is left, as Claude's and Codex's usage pages say it: `38% left`. Null when the bucket gives no fraction,
/// and for a window that has ended, whose figures are of the window before.
String? leftText(UsageBucket bucket, int now) {
  final used = bucket.fractionPermille;
  if (used == null || bucket.pastReset(now)) return null;
  return '${(1000 - used) ~/ 10}% left';
}

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

/// What an account's reports say: each bucket with how much is left, a bar, when it resets and the pace. The report is what the
/// sessions on the account last heard, so a figure says when it was observed.
class AccountUsageView extends StatelessWidget {
  const AccountUsageView({super.key, required this.report, this.now, this.warnPermille = defaultWarnPermille});

  final UsageReport? report;

  /// Unix seconds; the clock's when null.
  final int? now;
  final int warnPermille;

  @override
  Widget build(BuildContext context) {
    final at = now ?? DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final scheme = Theme.of(context).colorScheme;
    final small = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final buckets = report == null ? const <UsageBucket>[] : ordered(report!, at);
    if (buckets.isEmpty) {
      return Text(
        report == null ? 'No usage heard yet: it shows once a session has run on this account.' : 'Nothing reported yet',
        key: const Key('account-usage-empty'),
        style: small,
      );
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final bucket in buckets)
        Padding(
          key: Key('account-usage ${bucket.id}'),
          padding: const EdgeInsets.only(bottom: 12),
          child: _Bucket(bucket: bucket, now: at, warnPermille: warnPermille),
        ),
    ]);
  }
}

class _Bucket extends StatelessWidget {
  const _Bucket({required this.bucket, required this.now, required this.warnPermille});

  final UsageBucket bucket;
  final int now;
  final int warnPermille;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final small = text.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final left = leftText(bucket, now) ?? (bucket.isExhausted ? 'Limit reached' : figureOf(bucket));
    final level = levelOf(bucket, now, warnPermille: warnPermille);
    final colour = switch (level) {
      UsageLevel.danger => scheme.error,
      UsageLevel.warn => scheme.tertiary,
      UsageLevel.calm => scheme.primary,
    };
    final used = bucket.pastReset(now) ? null : bucket.fractionPermille;
    final reset = resetText(bucket, now), pace = forecastText(bucket, now);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Row(children: [
        Expanded(child: Text(bucket.name, style: text.bodyMedium)),
        if (left != null) Text(left, key: Key('account-left ${bucket.id}'), style: text.bodyMedium?.copyWith(color: colour)),
      ]),
      if (used != null)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(3),
            child: LinearProgressIndicator(value: (1000 - used) / 1000, minHeight: 6, color: colour, backgroundColor: scheme.surfaceContainerHighest),
          ),
        ),
      if (reset != null) Text(reset, key: Key('account-reset ${bucket.id}'), style: small),
      if (pace != null) Text(pace, key: Key('account-pace ${bucket.id}'), style: small?.copyWith(color: level == UsageLevel.calm ? null : colour)),
    ]);
  }
}
