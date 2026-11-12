import 'dart:async';

import 'package:flutter/material.dart';

import '../acp/usage_wire.dart';
import 'profiles_dialog.dart' show visible;

/// The host's two minutes to answer, less a margin for the answer to arrive.
const approvalWait = Duration(seconds: 110);

/// What the person chose when the host asked: the servers this session may start, and whether to keep that for the agent.
final class ApprovalChoice {
  const ApprovalChoice(this.allow, {this.remember = false});
  final List<String> allow;
  final bool remember;
}

/// The host's question before a session starts: each MCP server of an overlay or a project that canoa.cfg does not allow, the
/// layer it comes from and the digest of its definition, with a switch (off until the person allows it).
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
    title: Text('${widget.agent} asks to start MCP servers'),
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
        onPressed: () => Navigator.pop(context, ApprovalChoice(_on.toList(), remember: _remember)),
        child: const Text('Start'),
      ),
    ],
  );
}
