// Putting a session in a state a test needs without an agent: through the conversation's own methods, as an agent or a person would.
import 'dart:async';

import 'package:canoa/chat/session_options.dart' show optionToJson;
import 'package:canoa/chat/agent_events.dart' show AgentCommandInfo, ConfigOption;
import 'package:canoa/session_store.dart';
import 'package:coqui_acp/coqui_acp.dart';

extension SessionSetup on ChatSession {
  /// What the agent last said it offers.
  void offer(List<ConfigOption> options) => chat.applyOpened(
    AcpOpenedSession(remoteSessionId ?? id, {'configOptions': [for (final option in options) optionToJson(option)]}),
  );

  /// The commands the agent last listed.
  void offerCommands(List<AgentCommandInfo> commands) => chat.apply(
    AcpSessionUpdate(remoteSessionId ?? id, {
      'sessionUpdate': 'available_commands_update',
      'availableCommands': [for (final command in commands) {'name': command.name, 'description': command.description}],
    }),
  );

  /// Puts a request for an edit waiting for the person, as the agent's would.
  void asking([String title = 'Edit a.txt']) => unawaited(
    chat.askPermission(
      AcpPermissionRequest(
        sessionId: remoteSessionId ?? id,
        toolCallId: 'q${pendingPermissions.length}',
        title: title,
        kind: 'edit',
        rawInput: const {'file_path': 'a.txt'},
        options: const [AcpPermissionOption(optionId: 'allow', name: 'Yes', kind: 'allow_once')],
      ),
    ),
  );
}
