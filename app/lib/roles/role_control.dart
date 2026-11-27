import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatLookOf;
import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../session_store.dart';
import '../sources/sources.dart';
import 'role_models.dart';
import 'roles_controller.dart';

/// What a member is called: its agent as the sources name it, then its account, model and effort.
String memberText(RoleMember member, Iterable<AgentSource> sources, RolesController roles) {
  final named = sources.where((each) => (each.hostEntry ?? each.label) == member.agent).firstOrNull;
  return member.label(agentName: named?.displayName ?? roles.agentName(member.agent));
}

/// The words for why the host chose a member (`start`, `kept`, `in_turn`, `exhausted`); the host's own word when it is none of those.
String roleWhy(BuildContext context, String why) => AppText.of(context).maybe('role.why.$why') ?? why;

/// The Role control of a new session and of the picker: a draft chooses a role, or none, and is told which member a session of it
/// would run on and why; a running session says which role serves it and lets the person pick a member by hand, which ends the
/// role's hold on it. Nothing shows when the host has no roles.
class RoleControl extends StatefulWidget {
  const RoleControl({super.key, required this.store, required this.session, required this.roles, required this.sources});

  final SessionStore store;
  final ChatSession session;
  final RolesController roles;

  /// The agents the session can start from, which a member's agent is found among.
  final List<AgentSource> sources;

  @override
  State<RoleControl> createState() => _RoleControlState();
}

class _RoleControlState extends State<RoleControl> {
  String? _resolvedFor;
  RoleResolution? _resolution;
  String? _refusal;
  bool _asking = false;

  SessionStore get _store => widget.store;
  ChatSession get _session => widget.session;
  RolesController get _roles => widget.roles;

  @override
  void initState() {
    super.initState();
    unawaited(_roles.refresh());
  }

  /// Asks the host which member a new session of [role] would run on, and gives the draft that member's agent, account, model and
  /// effort. A refusal is kept to be said under the control.
  Future<void> _resolve(String role) async {
    _resolvedFor = role;
    setState(() {
      _asking = true;
      _refusal = null;
    });
    try {
      final resolved = await _roles.resolve(role);
      if (!mounted || _session.role != role) return;
      final title = _roles.named(role)?.label ?? role;
      final applied = _store.applyRole(_session, resolved, sources: widget.sources, title: title);
      setState(() {
        _resolution = resolved;
        _asking = false;
        _refusal = applied ? null : context.t('role.agent_missing', {'agent': resolved.member.agent});
      });
    } on RoleRefused catch (refused) {
      if (!mounted || _session.role != role) return;
      setState(() {
        _resolution = null;
        _asking = false;
        _refusal = refused.message;
      });
    } on Object catch (error) {
      if (!mounted || _session.role != role) return;
      setState(() {
        _resolution = null;
        _asking = false;
        _refusal = '$error';
      });
    }
  }

  void _choose(String role) {
    if (role.isEmpty) {
      _resolvedFor = null;
      setState(() {
        _resolution = null;
        _refusal = null;
      });
      _store.updateDraft(_session, noRole: true);
      return;
    }
    _store.updateDraft(_session, role: role);
    unawaited(_resolve(role));
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([_roles, _store]),
      builder: (context, _) {
        if (_session.isDraft) return _draft(context);
        return _running(context);
      },
    );
  }

  Widget _draft(BuildContext context) {
    final look = context.chatLook;
    final role = _session.role;
    if (_roles.roles.isEmpty && role == null) return const SizedBox.shrink();
    if (role != null && _resolvedFor != role && !_asking) unawaited(Future.microtask(() => mounted ? _resolve(role) : null));
    final chosen = role == null ? null : _roles.named(role);
    final resolution = _resolution;
    final line = resolution == null
        ? null
        : context.t('role.resolved', {
            'member': memberText(resolution.member, widget.sources, _roles),
            'why': roleWhy(context, resolution.why),
          });
    final skipped = resolution == null
        ? ''
        : [for (final each in resolution.skipped) '${each.member}: ${each.why}'].join('\n');
    return Column(crossAxisAlignment: CrossAxisAlignment.center, mainAxisSize: MainAxisSize.min, children: [
      PopupMenuButton<String>(
        key: const Key('role-control'),
        tooltip: context.t('role.tooltip'),
        onSelected: _choose,
        itemBuilder: (context) => [
          PopupMenuItem(key: const Key('role-option-none'), value: '', child: Text(context.t('role.none'))),
          for (final each in _roles.roles)
            CheckedPopupMenuItem(key: Key('role-option ${each.name}'), value: each.name, checked: each.name == role, child: Text(each.label)),
        ],
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          decoration: BoxDecoration(border: Border.all(color: look.muted.withValues(alpha: 0.35)), borderRadius: BorderRadius.circular(look.radius)),
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            Icon(Icons.alt_route, size: 18, color: look.muted),
            const SizedBox(width: 8),
            Flexible(child: Text(chosen?.label ?? role ?? context.t('role.label'), overflow: TextOverflow.ellipsis, maxLines: 1)),
            Icon(Icons.expand_more, size: 18, color: look.muted),
          ]),
        ),
      ),
      if (role != null)
        Padding(
          padding: const EdgeInsets.only(top: 6),
          child: _asking
              ? Text(context.t('role.resolving'), key: const Key('role-resolving'), style: TextStyle(color: look.muted, fontSize: 12.5))
              : _refusal != null
                  ? Text(_refusal!, key: const Key('role-refused'), textAlign: TextAlign.center, style: TextStyle(color: look.danger, fontSize: 12.5))
                  : line == null
                      ? const SizedBox.shrink()
                      : Tooltip(
                          message: skipped,
                          child: Text(
                            resolution!.usageKnown ? line : '$line (${context.t('role.usage_unreported')})',
                            key: const Key('role-resolved'),
                            textAlign: TextAlign.center,
                            style: TextStyle(color: look.muted, fontSize: 12.5),
                          ),
                        ),
        ),
    ]);
  }

  Widget _running(BuildContext context) {
    final look = context.chatLook;
    final serving = _session.roleServing;
    if (serving == null) return const SizedBox.shrink();
    return Container(
      key: const Key('role-running'),
      padding: const EdgeInsets.fromLTRB(0, 4, 0, 6),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(context.t('role.label'), style: TextStyle(fontSize: 11.5, color: look.muted)),
        const SizedBox(height: 2),
        Row(children: [
          Icon(Icons.alt_route, size: 16, color: look.muted),
          const SizedBox(width: 6),
          Flexible(
            child: Text(
              context.t('role.serves', {'role': serving.title, 'member': memberText(serving.member, widget.sources, _roles)}),
              key: const Key('role-serves'),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const SizedBox(width: 8),
          TextButton(
            key: const Key('role-by-hand'),
            onPressed: () => unawaited(_store.endRole(_session)),
            child: Text(context.t('role.by_hand')),
          ),
        ]),
      ]),
    );
  }
}
