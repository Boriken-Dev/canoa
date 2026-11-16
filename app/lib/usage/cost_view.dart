import 'package:flutter/material.dart';

import 'spend_view.dart';
import 'usage_model.dart';

/// Millionths of a US dollar as dollars: `$1.50`, and four places under a cent so a small cost does not read as none.
String dollars(int microUsd) {
  final value = microUsd / 1e6;
  return microUsd > 0 && value < 0.01 ? '\$${value.toStringAsFixed(4)}' : '\$${value.toStringAsFixed(2)}';
}

/// The Usage section's Cost view: the cost the agents reported, summed. Canoa has no price table, so nothing here is an estimate.
class CostView extends StatelessWidget {
  const CostView({super.key, required this.controller});

  final UsageController controller;

  @override
  Widget build(BuildContext context) => SpendView(
    controller: controller,
    metric: SpendMetric(
      name: 'Cost',
      columns: [
        MetricColumn(
          'Reported cost',
          'The cost, in US dollars, the agents said their turns came to.',
          (row) => row.costMicroUsd,
          dollars,
        ),
      ],
      chart: (row) => row.costMicroUsd,
      chartFormat: (value) => dollars(value.round()),
      note: 'Only the cost agents reported, as they reported it: nothing is estimated, and subscription fees are not included.',
    ),
  );
}
