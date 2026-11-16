import 'package:flutter/material.dart';

import '../accounts/usage_view.dart';
import 'usage_model.dart';

/// The allowances of each account the filters leave, under its provider: each bucket with how much is left, a bar, when it resets,
/// the pace and when it was read. An account several hosts reported is one card that says which hosts.
class LimitsView extends StatelessWidget {
  const LimitsView({
    super.key,
    required this.controller,
    required this.warnPermille,
  });

  final UsageController controller;
  final int warnPermille;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final small = text.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final shown = controller.visible;
    final providers = {for (final each in shown) each.provider}.toList();
    final now = controller.clock;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final MapEntry(:key, :value) in controller.problems.entries)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(
              '$key did not answer: $value',
              key: Key('usage-problem $key'),
              style: small?.copyWith(color: scheme.error),
            ),
          ),
        if (shown.isEmpty)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 24),
            child: Text(
              controller.loading
                  ? 'Asking the host…'
                  : controller.all.isEmpty
                  ? 'No limits reported yet: they show once a session has run on an account.'
                  : 'Nothing reported for this choice.',
              key: const Key('usage-limits-empty'),
              style: small,
            ),
          ),
        for (final provider in providers) ...[
          Padding(
            padding: const EdgeInsets.only(top: 8, bottom: 8),
            child: Text(
              provider,
              key: Key('usage-provider $provider'),
              style: text.titleSmall,
            ),
          ),
          for (final each in shown.where((each) => each.provider == provider))
            Card(
              key: Key('usage-account ${each.id}'),
              margin: const EdgeInsets.only(bottom: 12),
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(each.name, style: text.titleMedium),
                    Padding(
                      padding: const EdgeInsets.only(top: 2, bottom: 12),
                      child: Text(
                        each.hosts.isEmpty ? 'Not reported yet: it shows once its provider says, or a session has run on it' : 'Reported by ${each.hosts.join(', ')}',
                        key: Key('usage-hosts ${each.id}'),
                        style: small,
                      ),
                    ),
                    AccountUsageView(
                      report: each.report,
                      now: now,
                      warnPermille: warnPermille,
                      large: true,
                      readAt: each.readAt,
                    ),
                  ],
                ),
              ),
            ),
        ],
      ],
    );
  }
}
