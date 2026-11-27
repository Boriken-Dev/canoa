import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../accounts/accounts_controller.dart';
import '../desktop/control_client.dart' show hostWords;
import '../l10n/app_text.dart';
import '../settings/settings_row.dart';
import '../shell/confirm_remove.dart';
import 'role_served_line.dart' show clockTime;
import 'role_models.dart';
import 'roles_controller.dart';

/// The names a role may have: 1 to 32 lower-case letters, digits, hyphens and underscores.
final _roleName = RegExp(r'^[a-z0-9_-]{1,32}$');

/// Settings › Accounts › Roles: a role is a pool named by the work (planner, executor, reviewer, or any the person defines) whose
/// members are agents with an account, a model and an effort, taken in order. Each card says which member serves now, and why each of
/// the others cannot. Roles are written through the host's configuration store.
class RolesGroup extends StatefulWidget {
  const RolesGroup({super.key, required this.roles, required this.accounts});

  final RolesController roles;
  final AccountsController accounts;

  @override
  State<RolesGroup> createState() => _RolesGroupState();
}

class _RolesGroupState extends State<RolesGroup> {
  RolesController get _roles => widget.roles;

  @override
  void initState() {
    super.initState();
    if (!_roles.asked) unawaited(_roles.refresh());
  }

  Future<void> _edit([RoleInfo? role]) async {
    await showDialog<void>(context: context, builder: (_) => RoleDialog(roles: _roles, accounts: widget.accounts, role: role));
  }

  Future<void> _remove(RoleInfo role) async {
    final text = AppText.of(context);
    if (!await confirmRemoval(
      context,
      title: text('roles.remove_title', {'role': role.label}),
      body: text('roles.remove_body'),
      confirmLabel: text('roles.remove'),
      cancelLabel: text('roles.keep'),
      dialogKey: const Key('remove-role'),
      cancelKey: null,
      confirmKey: const Key('remove-role-confirm'),
    )) {
      return;
    }
    try {
      await _roles.remove(role.name);
    } on Object catch (error) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(hostWords(error), key: const Key('role-remove-problem'))));
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: _roles,
      builder: (context, _) {
        final text = AppText.of(context);
        // A host that cannot list roles (no grant for accounts, an older host) has none to show before the person makes one.
        if (_roles.problem != null && _roles.roles.isEmpty) {
          return SettingsGroup(
            key: const Key('roles'),
            title: text('roles.title'),
            leading: const Icon(Icons.alt_route, size: 20),
            children: [
              SettingsRow(title: text('roles.unavailable'), description: _roles.problem),
            ],
          );
        }
        return SettingsGroup(
          key: const Key('roles'),
          title: text('roles.title'),
          leading: const Icon(Icons.alt_route, size: 20),
          trailing: TextButton.icon(
            key: const Key('add-role'),
            onPressed: () => unawaited(_edit()),
            icon: const Icon(Icons.add, size: 18),
            label: Text(text('roles.add')),
          ),
          children: [
            SettingsRow(title: text('roles.intro_title'), description: text('roles.intro')),
            if (_roles.roles.isEmpty) SettingsRow(key: const Key('roles-none'), title: text('roles.none_title'), description: text('roles.none')),
            for (final role in _roles.roles) _row(context, role),
          ],
        );
      },
    );
  }

  Widget _row(BuildContext context, RoleInfo role) {
    final scheme = Theme.of(context).colorScheme;
    final text = AppText.of(context);
    final detail = [
      text('roles.strategy.${role.strategy}'),
      if (role.needs.isNotEmpty) text('roles.needs_line', {'needs': role.needs.join(', ')}),
      if (role.handover == 'ask') text('roles.handover_ask_line'),
    ].join(' · ');
    return SettingsRow(
      key: Key('role ${role.name}'),
      title: role.label,
      leading: const Icon(Icons.alt_route, size: 24),
      description: detail,
      below: Padding(
        padding: const EdgeInsets.only(top: 6),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          if (role.members.isEmpty) Text(text('roles.no_members'), style: Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant)),
          for (var i = 0; i < role.members.length; i++) _member(context, role, i),
          if (role.nothingLeft != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(role.nothingLeft!, key: Key('role-nothing-left ${role.name}'), style: TextStyle(color: scheme.error, fontSize: 12.5)),
            ),
        ]),
      ),
      control: Row(mainAxisSize: MainAxisSize.min, children: [
        IconButton(
          key: Key('role-edit ${role.name}'),
          tooltip: text('roles.edit'),
          icon: const Icon(Icons.edit_outlined, size: 20),
          onPressed: () => unawaited(_edit(role)),
        ),
        IconButton(
          key: Key('role-remove ${role.name}'),
          tooltip: text('roles.remove'),
          icon: Icon(Icons.delete_outline, size: 20, color: scheme.error),
          onPressed: () => unawaited(_remove(role)),
        ),
      ]),
    );
  }

  Widget _member(BuildContext context, RoleInfo role, int index) {
    final member = role.members[index];
    final scheme = Theme.of(context).colorScheme;
    final text = AppText.of(context);
    final skip = member.whyNot;
    final (label, color) = member.serving
        ? (text('roles.chip.serving'), scheme.primary)
        : skip == null
            ? (text('roles.chip.available'), scheme.onSurfaceVariant)
            : (
                [skip.why, if (skip.until != null) text('roles.chip.until', {'time': clockTime(DateTime.fromMillisecondsSinceEpoch(skip.until! * 1000))})].join(' · '),
                skip.unavailable ? scheme.error : scheme.tertiary,
              );
    return Padding(
      key: Key('role-member ${role.name} $index'),
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
        SizedBox(width: 22, child: Text('${index + 1}.', style: Theme.of(context).textTheme.bodySmall)),
        Expanded(child: Text(member.label(agentName: _roles.agentName(member.agent)), overflow: TextOverflow.ellipsis)),
        const SizedBox(width: 8),
        Flexible(
          child: Container(
            key: Key('role-chip ${role.name} $index'),
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
            decoration: BoxDecoration(border: Border.all(color: color.withValues(alpha: 0.6)), borderRadius: BorderRadius.circular(10)),
            child: Text(label, style: TextStyle(fontSize: 12, color: color), maxLines: 2, overflow: TextOverflow.ellipsis),
          ),
        ),
      ]),
    );
  }
}

/// One member being edited: its agent and account chosen, its model, effort and profile typed.
class _MemberDraft {
  _MemberDraft(RoleMember? from)
    : agent = from?.agent,
      account = from?.account,
      model = TextEditingController(text: from?.model ?? ''),
      effort = TextEditingController(text: from?.effort ?? ''),
      profile = TextEditingController(text: from?.profile ?? ''),
      priority = from?.priority ?? 0,
      maxConcurrent = from?.maxConcurrent,
      stopPermille = from?.stopPermille;

  String? agent, account;
  final TextEditingController model, effort, profile;

  /// What the dialog does not edit is kept as the host listed it.
  final int priority;
  final int? maxConcurrent, stopPermille;

  void dispose() {
    model.dispose();
    effort.dispose();
    profile.dispose();
  }

  RoleMember member() {
    String? text(TextEditingController each) => each.text.trim().isEmpty ? null : each.text.trim();
    return RoleMember(
      id: '',
      agent: agent ?? '',
      account: account,
      model: text(model),
      effort: text(effort),
      profile: text(profile),
      priority: priority,
      maxConcurrent: maxConcurrent,
      stopPermille: stopPermille,
    );
  }
}

/// Makes a role or changes one: its name (fixed once made), title, strategy, what its models must report, whether a session stays on
/// its member, the handover choice, and its members in the order they are taken. A refusal is shown in the host's own words.
class RoleDialog extends StatefulWidget {
  const RoleDialog({super.key, required this.roles, required this.accounts, this.role});

  final RolesController roles;
  final AccountsController accounts;

  /// The role being changed; null makes a new one.
  final RoleInfo? role;

  @override
  State<RoleDialog> createState() => _RoleDialogState();
}

class _RoleDialogState extends State<RoleDialog> {
  late final TextEditingController _name = TextEditingController(text: widget.role?.name ?? '');
  late final TextEditingController _title = TextEditingController(text: widget.role?.title ?? '');
  late final TextEditingController _needs = TextEditingController(text: widget.role?.needs.join(', ') ?? '');
  late final TextEditingController _resume = TextEditingController(text: widget.role?.resumePrompt ?? '');
  late String _strategy = widget.role?.strategy ?? 'priority';
  late String _handover = widget.role?.handover ?? 'auto';
  late bool _stick = widget.role?.stick ?? true;
  late bool _retry = widget.role?.retryTask ?? false;
  late final List<_MemberDraft> _members = [for (final each in widget.role?.members ?? const <RoleMember>[]) _MemberDraft(each)];
  bool _saving = false;
  String? _problem;

  @override
  void dispose() {
    for (final each in [_name, _title, _needs, _resume]) {
      each.dispose();
    }
    for (final each in _members) {
      each.dispose();
    }
    super.dispose();
  }

  /// The agents a member may be: what the app lists, and any a member already names.
  List<RoleAgent> _agents() {
    final listed = widget.roles.agents();
    return [
      ...listed,
      for (final member in _members)
        if (member.agent != null && !listed.any((each) => each.entry == member.agent)) (entry: member.agent!, name: member.agent!),
    ];
  }

  /// The accounts and pools [entry] offers, which a member may name; none means its first.
  List<String> _accountsOf(String entry, String? current) => {
    for (final each in widget.accounts.offeredBy(entry)) each.name,
    for (final each in widget.accounts.poolsOf(entry)) each.name,
    ?current,
  }.toList();

  void _move(int at, int by) => setState(() {
    final to = at + by;
    if (to < 0 || to >= _members.length) return;
    _members.insert(to, _members.removeAt(at));
  });

  void _removeMember(int at) => setState(() => _members.removeAt(at).dispose());

  Future<void> _save() async {
    final text = AppText.of(context);
    final name = _name.text.trim();
    if (!_roleName.hasMatch(name)) {
      setState(() => _problem = text('roles.name_rule'));
      return;
    }
    if (_members.any((each) => each.agent == null || each.agent!.isEmpty)) {
      setState(() => _problem = text('roles.member_needs_agent'));
      return;
    }
    final needs = [for (final each in _needs.text.split(',')) if (each.trim().isNotEmpty) each.trim()];
    final resume = _resume.text.trim();
    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      await widget.roles.save(RoleInfo(
        name: name,
        title: _title.text.trim(),
        strategy: _strategy,
        needs: needs,
        stick: _stick,
        handover: _handover,
        resumePrompt: resume.isEmpty ? null : resume,
        retryTask: _retry,
        members: [for (final each in _members) each.member()],
      ));
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
    final text = AppText.of(context);
    final muted = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    final agents = _agents();
    return AlertDialog(
      key: const Key('role-dialog'),
      title: Text(widget.role == null ? text('roles.add') : text('roles.edit_title', {'role': widget.role!.label})),
      content: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560),
        child: SizedBox(
          width: double.maxFinite,
          child: SingleChildScrollView(
            child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              TextField(
                key: const Key('role-name'),
                controller: _name,
                enabled: widget.role == null && !_saving,
                decoration: InputDecoration(labelText: text('roles.name'), helperText: text('roles.name_rule')),
              ),
              const SizedBox(height: 8),
              TextField(key: const Key('role-title'), controller: _title, enabled: !_saving, decoration: InputDecoration(labelText: text('roles.title_field'))),
              const SizedBox(height: 12),
              DropdownButtonFormField<String>(
                key: const Key('role-strategy'),
                initialValue: _strategy,
                isExpanded: true,
                decoration: InputDecoration(labelText: text('roles.strategy')),
                items: [
                  for (final each in const ['priority', 'headroom', 'round_robin', 'least_recent'])
                    DropdownMenuItem(value: each, child: Text(text('roles.strategy.$each'))),
                ],
                onChanged: _saving ? null : (value) => setState(() => _strategy = value ?? _strategy),
              ),
              const SizedBox(height: 8),
              TextField(
                key: const Key('role-needs'),
                controller: _needs,
                enabled: !_saving,
                decoration: InputDecoration(labelText: text('roles.needs'), helperText: text('roles.needs_help')),
              ),
              SwitchListTile(
                key: const Key('role-stick'),
                contentPadding: EdgeInsets.zero,
                title: Text(text('roles.stick')),
                subtitle: Text(text('roles.stick_help')),
                value: _stick,
                onChanged: _saving ? null : (value) => setState(() => _stick = value),
              ),
              DropdownButtonFormField<String>(
                key: const Key('role-handover'),
                initialValue: _handover,
                isExpanded: true,
                decoration: InputDecoration(labelText: text('roles.handover'), helperText: text('roles.handover_help')),
                items: [
                  for (final each in const ['auto', 'ask']) DropdownMenuItem(value: each, child: Text(text('roles.handover.$each'))),
                ],
                onChanged: _saving ? null : (value) => setState(() => _handover = value ?? _handover),
              ),
              const SizedBox(height: 8),
              TextField(
                key: const Key('role-resume'),
                controller: _resume,
                enabled: !_saving,
                decoration: InputDecoration(labelText: text('roles.resume'), helperText: text('roles.resume_help')),
              ),
              SwitchListTile(
                key: const Key('role-retry'),
                contentPadding: EdgeInsets.zero,
                title: Text(text('roles.retry')),
                subtitle: Text(text('roles.retry_help')),
                value: _retry,
                onChanged: _saving ? null : (value) => setState(() => _retry = value),
              ),
              const SizedBox(height: 8),
              Text(text('roles.members'), style: muted),
              for (var i = 0; i < _members.length; i++) _memberEditor(context, i, agents),
              Align(
                alignment: Alignment.centerLeft,
                child: TextButton.icon(
                  key: const Key('role-add-member'),
                  onPressed: _saving ? null : () => setState(() => _members.add(_MemberDraft(null)..agent = agents.firstOrNull?.entry)),
                  icon: const Icon(Icons.add, size: 18),
                  label: Text(text('roles.add_member')),
                ),
              ),
              if (_problem case final problem?)
                Padding(padding: const EdgeInsets.only(top: 10), child: Text(problem, key: const Key('role-problem'), style: TextStyle(color: scheme.error))),
            ]),
          ),
        ),
      ),
      actions: [
        TextButton(key: const Key('role-cancel'), onPressed: _saving ? null : () => Navigator.pop(context), child: Text(text('roles.cancel'))),
        FilledButton(key: const Key('role-save'), onPressed: _saving ? null : () => unawaited(_save()), child: Text(text('roles.save'))),
      ],
    );
  }

  Widget _memberEditor(BuildContext context, int at, List<RoleAgent> agents) {
    final member = _members[at];
    final text = AppText.of(context);
    final scheme = Theme.of(context).colorScheme;
    final entry = member.agent;
    final accounts = entry == null ? const <String>[] : _accountsOf(entry, member.account);
    return Container(
      key: Key('role-member-editor $at'),
      margin: const EdgeInsets.only(top: 8),
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(border: Border.all(color: scheme.outlineVariant), borderRadius: BorderRadius.circular(8)),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Row(children: [
          Expanded(child: Text(text('roles.member_n', {'n': at + 1}), style: Theme.of(context).textTheme.labelMedium)),
          IconButton(
            key: Key('role-member-up $at'),
            tooltip: text('roles.earlier'),
            icon: const Icon(Icons.arrow_upward, size: 18),
            onPressed: _saving || at == 0 ? null : () => _move(at, -1),
          ),
          IconButton(
            key: Key('role-member-down $at'),
            tooltip: text('roles.later'),
            icon: const Icon(Icons.arrow_downward, size: 18),
            onPressed: _saving || at == _members.length - 1 ? null : () => _move(at, 1),
          ),
          IconButton(
            key: Key('role-member-remove $at'),
            tooltip: text('roles.remove_member'),
            icon: Icon(Icons.delete_outline, size: 18, color: scheme.error),
            onPressed: _saving ? null : () => _removeMember(at),
          ),
        ]),
        DropdownButtonFormField<String>(
          key: ValueKey('role-member-agent $at ${member.agent}'),
          initialValue: agents.any((each) => each.entry == member.agent) ? member.agent : null,
          isExpanded: true,
          decoration: InputDecoration(labelText: text('roles.agent'), isDense: true),
          items: [for (final each in agents) DropdownMenuItem(value: each.entry, child: Text(each.name, overflow: TextOverflow.ellipsis))],
          onChanged: _saving
              ? null
              : (value) => setState(() {
                  member
                    ..agent = value
                    ..account = null;
                }),
        ),
        const SizedBox(height: 8),
        DropdownButtonFormField<String>(
          key: ValueKey('role-member-account $at ${member.agent} ${member.account}'),
          initialValue: member.account ?? '',
          isExpanded: true,
          decoration: InputDecoration(labelText: text('roles.account'), isDense: true),
          items: [
            DropdownMenuItem(value: '', child: Text(text('roles.account_first'))),
            for (final each in accounts) DropdownMenuItem(value: each, child: Text(each, overflow: TextOverflow.ellipsis)),
          ],
          onChanged: _saving ? null : (value) => setState(() => member.account = value == null || value.isEmpty ? null : value),
        ),
        const SizedBox(height: 8),
        LayoutBuilder(builder: (context, box) {
          // Three fields side by side where they fit, one under another where they do not.
          final width = box.maxWidth < 420 ? box.maxWidth : (box.maxWidth - 16) / 3;
          Widget field(Key key, TextEditingController controller, String label) => SizedBox(
            width: width,
            child: TextField(key: key, controller: controller, enabled: !_saving, decoration: InputDecoration(labelText: label, isDense: true)),
          );
          return Wrap(spacing: 8, runSpacing: 8, children: [
            field(Key('role-member-model $at'), member.model, text('roles.model')),
            field(Key('role-member-effort $at'), member.effort, text('roles.effort')),
            field(Key('role-member-profile $at'), member.profile, text('roles.profile')),
          ]);
        }),
      ]),
    );
  }
}
