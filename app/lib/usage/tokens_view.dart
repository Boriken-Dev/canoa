import 'package:material_ui/material_ui.dart';

import 'spend_view.dart';
import 'usage_model.dart';

/// The tokens each view counts, each with what it means.
final tokenColumns = [
  MetricColumn(
    'Input (uncached)',
    'Tokens the model read that were not served from the cache.',
    (row) => row.input,
    groupDigits,
  ),
  MetricColumn(
    'Cache read',
    'Tokens the model read from the cache, which the provider counts apart from fresh input.',
    (row) => row.cacheRead,
    groupDigits,
  ),
  MetricColumn(
    'Cache write',
    'Tokens written into the cache for later turns to read.',
    (row) => row.cacheWrite,
    groupDigits,
  ),
  MetricColumn(
    'Output',
    'Tokens the model wrote, including any reasoning the agent counts with them.',
    (row) => row.output,
    groupDigits,
  ),
];

/// A count as the chart's axis says it: 1.2k, 3.4M.
String compactTokens(double value) {
  if (value >= 1e9) return '${(value / 1e9).toStringAsFixed(1)}B';
  if (value >= 1e6) return '${(value / 1e6).toStringAsFixed(1)}M';
  if (value >= 1e3) return '${(value / 1e3).toStringAsFixed(1)}k';
  return value.round().toString();
}

/// The Usage section's Tokens view: the four token counters the agents reported, over a range.
class TokensView extends StatelessWidget {
  const TokensView({super.key, required this.controller});

  final UsageController controller;

  @override
  Widget build(BuildContext context) => SpendView(
    controller: controller,
    metric: SpendMetric(
      name: 'Tokens',
      columns: tokenColumns,
      chart: (row) => row.tokens,
      chartFormat: compactTokens,
      note: 'The chart and the account shares add the four counters; a counter an agent did not report is left out, not counted as none.',
    ),
  );
}
