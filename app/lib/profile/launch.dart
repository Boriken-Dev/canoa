import '../acp/agent_commands.dart';
import '../acp/client.dart';
import '../acp/local_agent.dart';
import 'apply.dart';

/// Starts an agent on this computer with the environment of [prepared], as [startLocalAgent] does.
typedef StartAgent = Future<VerifiedAgent> Function(
  AgentCommand command, {
  AgentDecider? decider,
  AgentElicitor? elicitor,
  String? workingDirectory,
  Map<String, String> environment,
});

/// An agent started with a profile, and the launch it is to open its session with.
final class ProfiledAgent {
  const ProfiledAgent(this.client, this.launch, this.harness);
  final VerifiedAgent client;
  final ProfileLaunch launch;

  /// The harness id when Canoa knows it, else the agent entry's label.
  final String harness;
}

/// Starts [command] in [cwd] with [prepared]'s environment, then works out from what the agent says in `initialize` how
/// it takes the rest: [mcpOn] names the MCP servers the person allowed. What a harness reads from a directory is written
/// through [files], and removed if the agent cannot be started. The caller opens the session with the launch's setup.
Future<ProfiledAgent> startProfiled(
  AgentCommand command, {
  required String cwd,
  required PreparedProfile prepared,
  Set<String> mcpOn = const {},
  SessionFiles? files,
  AgentDecider? decider,
  AgentElicitor? elicitor,
  StartAgent start = startLocalAgent,
}) async {
  final client = await start(
    command,
    decider: decider,
    elicitor: elicitor,
    workingDirectory: cwd,
    environment: {for (final MapEntry(:key, :value) in prepared.environment.entries) key: value.expose()},
  );
  try {
    final harness = harnessIdOf(reportedName: client.identity.name, grokShell: client.identity.grokShell, entry: command.label);
    final launch = await launchFor(
      prepared,
      harness: harness,
      takes: AgentTakes(
        mcp: client.takesMcp,
        systemPrompt: client.takesSystemPrompt,
        additionalDirectories: client.takesAdditionalDirectories,
      ),
      mcpOn: mcpOn,
      files: files,
    );
    if (launch.temporary case final dir? when files != null) client.afterClose(() => files.remove(dir));
    return ProfiledAgent(client, launch, harness ?? command.label);
  } catch (_) {
    await client.close();
    rethrow;
  }
}
