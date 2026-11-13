import 'dart:async';

import 'package:flutter/material.dart';

import '../acp/usage_wire.dart';
import 'profiles_panel.dart' show visible;

/// The host's two minutes to answer, less a margin for the answer to arrive.
const approvalWait = Duration(seconds: 110);

/// What the person chose when the host asked: the servers this session may start and the overlays whose hooks it may run, and
/// whether to keep that for the agent.
final class ApprovalChoice {
  const ApprovalChoice(this.allow, {this.hooks = const [], this.remember = false});
  final List<String> allow;
  final List<String> hooks;
  final bool remember;
}

/// The host's question before a session starts: each MCP server of an overlay or a project that canoa.cfg does not allow, the
/// layer it comes from and the digest of its definition, and each overlay's hooks, the events they are bound to and the digest of
/// the overlay's files, each with a switch (off until the person allows it).
class ProfileApprovalDialog extends StatefulWidget {
  const ProfileApprovalDialog({super.key, required this.ask, required this.agent, this.expires = approvalWait});
  final ProfileApproval ask;
  final String agent;

  /// How long the question stays open: the host gives up on it after two minutes, and an answer later counts for nothing.
  final Duration expires;

  @override
  State<ProfileApprovalDialog> createState() => _ProfileApprovalDialogState();
}

class _ProfileApprovalDialogState extends State<ProfileApprovalDialog> {
  final _on = <String>{};
  final _hooksOn = <String>{};
  bool _remember = false;
  Timer? _expiry;

  @override
  void initState() {
    super.initState();
    // Closing without an answer allows nothing and keeps nothing.
    _expiry = Timer(widget.expires, () {
      if (mounted) Navigator.pop(context);
    });
  }

  @override
  void dispose() {
    _expiry?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text('${widget.agent} asks to start ${_what()}'),
    content: SizedBox(
      width: 560,
      child: SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
          const Text('These come from an overlay or a project, and run on this computer once started. Nothing starts that '
              'is not switched on here.'),
          for (final server in widget.ask.servers)
            SwitchListTile(
              key: Key('approve-${server.name}'),
              title: Text(visible(server.name)),
              subtitle: Text(visible('${server.layer}\n${server.digest}')),
              value: _on.contains(server.name),
              onChanged: (value) => setState(() => value ? _on.add(server.name) : _on.remove(server.name)),
            ),
          if (widget.ask.hooks.isNotEmpty) ...[
            const Padding(
              padding: EdgeInsets.only(top: 12),
              child: Text('Hooks are programs an overlay runs at moments of the session. They run outside the agent\'s '
                  'sandbox with your access, can read what you type when they are bound to the prompt, can add to the '
                  'agent\'s context, and can stop a prompt or a tool call, but never approve one. Allowing them covers '
                  'exactly these files: any change asks again.'),
            ),
            for (final hook in widget.ask.hooks)
              SwitchListTile(
                key: Key('approve-hooks-${hook.overlay}'),
                title: Text(visible('Hooks of ${hook.overlay}')),
                subtitle: Text(visible(_describe(hook))),
                value: _hooksOn.contains(hook.overlay),
                onChanged: (value) => setState(() => value ? _hooksOn.add(hook.overlay) : _hooksOn.remove(hook.overlay)),
              ),
          ],
          CheckboxListTile(
            key: const Key('approve-remember'),
            title: Text('Keep these choices for ${widget.agent}'),
            subtitle: const Text('Written to canoa.cfg, and used the next time the host starts; a server whose definition changes asks again.'),
            value: _remember,
            onChanged: (value) => setState(() => _remember = value ?? false),
          ),
        ]),
      ),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context, const ApprovalChoice([])), child: const Text('Start none')),
      FilledButton(
        key: const Key('approve-start'),
        onPressed: () => Navigator.pop(context, ApprovalChoice(_on.toList(), hooks: _hooksOn.toList(), remember: _remember)),
        child: const Text('Start'),
      ),
    ],
  );

  String _what() => switch ((widget.ask.servers.isNotEmpty, widget.ask.hooks.isNotEmpty)) {
    (true, true) => 'MCP servers and hooks',
    (false, true) => 'hooks',
    _ => 'MCP servers',
  };

  /// What a hook switch says: the digest it would be allowed at, and each event with its script and how a failure counts.
  static String _describe(ApprovalHook hook) {
    final lines = <String>[
      hook.digest,
      if (hook.changed) 'Allowed before at another version of these files.',
      if (hook.failClosed) 'A hook here refuses the event when it fails, so the overlay is not used without it.',
      for (final each in hook.events)
        '${each.event}: ${each.run ?? 'no script can run here'}'
            '${each.match.isEmpty ? '' : ' for ${each.match.join(', ')}'}'
            '${each.timeout == null ? '' : ', ${each.timeout} s at most'}'
            '${each.onFailure == 'refuse' ? ', refuses on failure' : ''}'
            '${each.sets.isEmpty ? '' : ', may set ${each.sets.join(', ')} (not applied over ACP)'}'
            '${each.fires ? '' : ' (${each.note ?? 'Canoa does not see this event, so it never runs'})'}',
    ];
    return lines.join('\n');
  }
}
