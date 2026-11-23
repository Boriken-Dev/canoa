import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../settings/agent_sections.dart' show WarnFromRow;
import '../settings/settings_row.dart' show SettingsGroup;
import '../sources/source_icons.dart';
import 'cost_view.dart';
import 'limits_view.dart';
import 'spend_model.dart';
import 'tokens_view.dart';
import 'usage_model.dart';

/// The Usage section's index: All accounts, then each provider with its accounts. What it selects is what the page's filters show.
class UsageIndex extends StatelessWidget {
  const UsageIndex({super.key, required this.controller});

  final UsageController controller;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) {
      final scheme = Theme.of(context).colorScheme;
      final nothing = controller.provider == null && controller.account == null;
      return ListView(
        padding: const EdgeInsets.symmetric(vertical: 8),
        children: [
          ListTile(
            key: const Key('usage-index all'),
            dense: true,
            leading: const Icon(Icons.stacked_bar_chart, size: 20),
            title: const Text('All accounts'),
            selected: nothing,
            onTap: () => controller.select(host: controller.host),
          ),
          for (final provider in controller.providers) ...[
            ListTile(
              key: Key('usage-index provider $provider'),
              dense: true,
              leading: SourceMark.provider(provider, size: 18, key: Key('usage-index-mark $provider')),
              title: Text(
                provider,
                style: Theme.of(context).textTheme.labelMedium?.copyWith(
                  color: scheme.onSurfaceVariant,
                ),
              ),
              selected:
                  controller.provider == provider && controller.account == null,
              onTap: () =>
                  controller.select(host: controller.host, provider: provider),
            ),
            for (final each in controller.of(provider))
              ListTile(
                key: Key('usage-index account ${each.id}'),
                dense: true,
                contentPadding: const EdgeInsets.only(left: 28, right: 12),
                leading: SourceMark.provider(each.provider, size: 18, key: Key('usage-index-mark ${each.id}')),
                title: Text(each.name),
                selected: controller.account == each.id,
                onTap: () =>
                    controller.select(host: controller.host, account: each.id),
              ),
          ],
        ],
      );
    },
  );
}

/// The Usage section's main view: the Host, Provider and Account filters over the figures. Limits is the first of its views.
/// Accounts are managed in Settings, which [onManageAccounts] opens.
class UsagePage extends StatefulWidget {
  const UsagePage({
    super.key,
    required this.controller,
    required this.warnPermille,
    required this.onWarnChanged,
    this.onManageAccounts,
  });

  final UsageController controller;
  final int warnPermille;
  final ValueChanged<int> onWarnChanged;
  final VoidCallback? onManageAccounts;

  @override
  State<UsagePage> createState() => _UsagePageState();
}

class _UsagePageState extends State<UsagePage> {
  UsageController get _usage => widget.controller;

  @override
  void initState() {
    super.initState();
    // Asking notifies, which a build in progress cannot take: ask once this frame is done.
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => unawaited(_usage.refresh()),
    );
  }

  Widget _filter(
    String label,
    Key key,
    String? value,
    List<(String, String)> choices,
    ValueChanged<String?> onChanged,
  ) => SizedBox(
    width: 200,
    child: InputDecorator(
      decoration: InputDecoration(
        labelText: label,
        isDense: true,
        border: const OutlineInputBorder(),
      ),
      child: DropdownButtonHideUnderline(
        child: DropdownButton<String?>(
          key: key,
          value: choices.any((each) => each.$1 == value) ? value : null,
          isExpanded: true,
          isDense: true,
          items: [
            DropdownMenuItem<String?>(
              value: null,
              child: Text('All ${label.toLowerCase()}s'),
            ),
            for (final (id, name) in choices)
              DropdownMenuItem<String?>(
                value: id,
                child: Text(name, overflow: TextOverflow.ellipsis),
              ),
          ],
          onChanged: onChanged,
        ),
      ),
    ),
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _usage,
    builder: (context, _) {
      final text = Theme.of(context).textTheme;
      return Align(
        alignment: Alignment.topCenter,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 760),
          // The figures, labels, reasons and errors are text a person copies.
          child: SelectionArea(child: ListView(
            padding: const EdgeInsets.all(16),
            children: [
              Row(
                children: [
                  Expanded(child: Text('Usage', style: text.headlineSmall)),
                  IconButton(
                    key: const Key('usage-refresh'),
                    tooltip: _usage.view == UsageView.limits ? 'Read the limits again' : 'Read the figures again',
                    icon: _usage.loading
                        ? const SizedBox(
                            width: 18,
                            height: 18,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Icon(Icons.refresh),
                    onPressed: _usage.loading
                        ? null
                        : () => unawaited(_usage.view == UsageView.limits ? _usage.refresh() : Future.wait([_usage.refresh(), _usage.refreshSpend()])),
                  ),
                ],
              ),
              SegmentedButton<UsageView>(
                key: const Key('usage-view'),
                showSelectedIcon: false,
                segments: [
                  for (final each in UsageView.values)
                    ButtonSegment(value: each, label: Text(each.label), tooltip: each.label),
                ],
                selected: {_usage.view},
                onSelectionChanged: (chosen) => _usage.setView(chosen.first),
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 12,
                runSpacing: 12,
                children: [
                  if (_usage.sources.length > 1)
                    _filter(
                      'Host',
                      const Key('usage-filter host'),
                      _usage.host,
                      [for (final s in _usage.sources) (s.name, s.name)],
                      _usage.selectHost,
                    ),
                  _filter(
                    'Provider',
                    const Key('usage-filter provider'),
                    _usage.provider,
                    [for (final p in _usage.providers) (p, p)],
                    _usage.selectProvider,
                  ),
                  _filter(
                    'Account',
                    const Key('usage-filter account'),
                    _usage.account,
                    [
                      for (final each in _usage.all)
                        if (_usage.provider == null ||
                            each.provider == _usage.provider)
                          (each.id, each.name),
                    ],
                    _usage.selectAccount,
                  ),
                ],
              ),
              const SizedBox(height: 16),
              switch (_usage.view) {
                UsageView.limits => LimitsView(
                  controller: _usage,
                  warnPermille: widget.warnPermille,
                ),
                UsageView.tokens => TokensView(controller: _usage),
                UsageView.cost => CostView(controller: _usage),
              },
              const SizedBox(height: 16),
              if (_usage.view == UsageView.limits)
                SettingsGroup(
                  title: 'Warning',
                  children: [
                    WarnFromRow(
                      value: widget.warnPermille,
                      onChanged: widget.onWarnChanged,
                    ),
                  ],
                ),
              if (widget.onManageAccounts != null)
                Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton.icon(
                    key: const Key('usage-manage-accounts'),
                    onPressed: widget.onManageAccounts,
                    icon: const Icon(Icons.manage_accounts_outlined, size: 18),
                    label: const Text('Manage accounts in Settings'),
                  ),
                ),
            ],
          )),
        ),
      );
    },
  );
}
