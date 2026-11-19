import 'dart:async';

import 'package:flutter/material.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../settings/settings_row.dart';
import 'account_models.dart';
import 'accounts_controller.dart';

/// The host's words for why a request failed: the refusal as the host wrote it, with nothing the app adds.
String hostWords(Object error) => error is ControlFailed ? error.message : '$error'.replaceFirst(RegExp(r'^(Bad state|Exception): '), '');

/// Settings › Accounts › Pools: sets of accounts that provide one credential standard, which a session may run on instead of one
/// account, on whichever has usage left. The person makes, edits and removes their own; the host makes one for every standard
/// an account provides (`any-<standard>`), which lists every such account and is read-only here.
class PoolsGroup extends StatefulWidget {
  const PoolsGroup({super.key, required this.controller});

  final AccountsController controller;

  @override
  State<PoolsGroup> createState() => _PoolsGroupState();
}

class _PoolsGroupState extends State<PoolsGroup> {
  AccountsController get _accounts => widget.controller;

  Future<void> _edit([PoolInfo? pool]) async {
    await showDialog<void>(context: context, builder: (_) => PoolDialog(accounts: _accounts, pool: pool));
  }

  Future<void> _remove(PoolInfo pool) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('remove-pool'),
        title: Text('Remove ${pool.name}?'),
        content: const Text('The accounts in it stay as they are; only the set goes, and the sources that offered it stop offering it.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep it')),
          FilledButton(key: const Key('remove-pool-confirm'), onPressed: () => Navigator.pop(context, true), child: const Text('Remove')),
        ],
      ),
    );
    if (sure != true) return;
    try {
      await _accounts.removePool(pool.name);
    } on Object catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(hostWords(error), key: const Key('pool-remove-problem'))));
    }
  }

  @override
  Widget build(BuildContext context) {
    final pools = _accounts.pools;
    // The host lists a pool of every standard some account provides, so a host that lists none has no account to pool yet.
    if (pools.isEmpty) return const SizedBox.shrink();
    return SettingsGroup(
      key: const Key('pools'),
      title: 'Pools',
      leading: const Icon(Icons.layers_outlined, size: 20),
      trailing: TextButton.icon(
        key: const Key('add-pool'),
        onPressed: _accounts.standards.isEmpty ? null : () => unawaited(_edit()),
        icon: const Icon(Icons.add, size: 18),
        label: const Text('Make a pool'),
      ),
      children: [
        const SettingsRow(
          title: 'Accounts that work together',
          description: 'A pool holds accounts that provide one standard. A session picks a pool as it would an account, and runs on one of '
              'them. Canoa makes a pool of every standard some account provides: it follows the accounts as they come and go.',
        ),
        for (final pool in pools) _row(pool),
      ],
    );
  }

  Widget _row(PoolInfo pool) {
    final scheme = Theme.of(context).colorScheme;
    final members = pool.accounts.map((each) => each == localAccount ? 'the login on this computer' : each).join(', ');
    return SettingsRow(
      key: Key('pool ${pool.name}'),
      title: pool.name,
      leading: const Icon(Icons.layers_outlined, size: 24),
      description: [
        standardTitle(pool.standard),
        if (pool.automatic) 'Made by Canoa',
        members.isEmpty ? 'no account yet' : members,
      ].join(' · '),
      below: pool.automatic
          ? Text(
              'Every account that provides ${standardTitle(pool.standard)}; nothing to edit.',
              key: Key('pool-automatic ${pool.name}'),
              style: Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant),
            )
          : null,
      control: pool.automatic
          ? null
          : Row(mainAxisSize: MainAxisSize.min, children: [
              IconButton(key: Key('pool-edit ${pool.name}'), tooltip: 'Edit', icon: const Icon(Icons.edit_outlined, size: 20), onPressed: () => unawaited(_edit(pool))),
              IconButton(
                key: Key('pool-remove ${pool.name}'),
                tooltip: 'Remove',
                icon: Icon(Icons.delete_outline, size: 20, color: scheme.error),
                onPressed: () => unawaited(_remove(pool)),
              ),
            ]),
    );
  }
}

/// Makes a pool or changes one: its name (fixed once made), the standard its accounts provide, and the accounts, which the
/// section offers exactly as those that provide that standard. The order is the one the pool takes them in. A refusal is shown
/// in the host's own words.
class PoolDialog extends StatefulWidget {
  const PoolDialog({super.key, required this.accounts, this.pool});

  final AccountsController accounts;

  /// The pool being changed; null makes a new one.
  final PoolInfo? pool;

  @override
  State<PoolDialog> createState() => _PoolDialogState();
}

class _PoolDialogState extends State<PoolDialog> {
  late final TextEditingController _name = TextEditingController(text: widget.pool?.name ?? '');
  late String? _standard = widget.pool?.standard ?? widget.accounts.standards.firstOrNull;
  late List<String> _picked = [...?widget.pool?.accounts];
  bool _saving = false;
  String? _problem;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  /// What the pool may hold now: the accounts that provide the standard, the person's choice first and in its order.
  List<String> get _offered {
    final standard = _standard;
    if (standard == null) return const [];
    final all = widget.accounts.membersFor(standard);
    return [
      for (final each in _picked)
        if (all.contains(each)) each,
      for (final each in all)
        if (!_picked.contains(each)) each,
    ];
  }

  void _setStandard(String? standard) {
    if (standard == null) return;
    setState(() {
      _standard = standard;
      final all = widget.accounts.membersFor(standard);
      _picked = [for (final each in _picked) if (all.contains(each)) each];
    });
  }

  void _toggle(String name, bool on) => setState(() => on ? _picked.add(name) : _picked.remove(name));

  void _move(String name, int by) => setState(() {
    final at = _picked.indexOf(name);
    final to = at + by;
    if (at < 0 || to < 0 || to >= _picked.length) return;
    _picked
      ..removeAt(at)
      ..insert(to, name);
  });

  Future<void> _save() async {
    final standard = _standard, name = _name.text.trim();
    if (standard == null || name.isEmpty) {
      setState(() => _problem = 'A pool has a name and a standard.');
      return;
    }
    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      await widget.accounts.setPool(name, standard, _picked);
      if (mounted) Navigator.pop(context);
    } on Object catch (error) {
      if (mounted) {
        setState(() {
          _saving = false;
          _problem = hostWords(error);
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final muted = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final offered = _offered;
    return AlertDialog(
      key: const Key('pool-dialog'),
      title: Text(widget.pool == null ? 'Make a pool' : 'Edit ${widget.pool!.name}'),
      content: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 460),
        child: SizedBox(
          width: double.maxFinite,
          child: SingleChildScrollView(
            child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              TextField(
                key: const Key('pool-name'),
                controller: _name,
                enabled: widget.pool == null && !_saving,
                decoration: const InputDecoration(labelText: 'Name'),
              ),
              const SizedBox(height: 12),
              DropdownButtonFormField<String>(
                key: const Key('pool-standard'),
                initialValue: _standard,
                decoration: const InputDecoration(labelText: 'Accounts that provide'),
                items: [
                  for (final each in widget.accounts.standards) DropdownMenuItem(value: each, child: Text(standardTitle(each))),
                ],
                onChanged: _saving ? null : _setStandard,
              ),
              const SizedBox(height: 12),
              Text('Accounts, in the order the pool takes them', style: muted),
              if (offered.isEmpty) Padding(padding: const EdgeInsets.only(top: 8), child: Text('No account provides this standard yet.', key: const Key('pool-none'), style: muted)),
              for (final name in offered)
                Row(key: Key('pool-member $name'), children: [
                  Checkbox(key: Key('pool-check $name'), value: _picked.contains(name), onChanged: _saving ? null : (on) => _toggle(name, on ?? false)),
                  Expanded(child: Text(name == localAccount ? 'The login on this computer' : name, overflow: TextOverflow.ellipsis)),
                  if (_picked.contains(name)) ...[
                    IconButton(
                      key: Key('pool-up $name'),
                      tooltip: 'Earlier',
                      icon: const Icon(Icons.arrow_upward, size: 18),
                      onPressed: _saving || _picked.indexOf(name) == 0 ? null : () => _move(name, -1),
                    ),
                    IconButton(
                      key: Key('pool-down $name'),
                      tooltip: 'Later',
                      icon: const Icon(Icons.arrow_downward, size: 18),
                      onPressed: _saving || _picked.indexOf(name) == _picked.length - 1 ? null : () => _move(name, 1),
                    ),
                  ],
                ]),
              if (_problem case final problem?)
                Padding(padding: const EdgeInsets.only(top: 10), child: Text(problem, key: const Key('pool-problem'), style: TextStyle(color: scheme.error))),
            ]),
          ),
        ),
      ),
      actions: [
        TextButton(key: const Key('pool-cancel'), onPressed: _saving ? null : () => Navigator.pop(context), child: const Text('Cancel')),
        FilledButton(key: const Key('pool-save'), onPressed: _saving ? null : () => unawaited(_save()), child: const Text('Save')),
      ],
    );
  }
}
