import 'agent_commands.dart';
import 'client.dart';

/// A page cannot start a program.
const bool canRunLocalAgents = false;

Future<bool> isInstalled(AgentCommand command) async => false;

String? defaultWorkingDirectory() => null;

Future<String?> findProgram(String program) async => null;

String? environmentVariable(String name) => null;

Future<VerifiedAgent> startLocalAgent(
  AgentCommand command, {
  AgentDecider? decider,
  AgentElicitor? elicitor,
  String? workingDirectory,
  Map<String, String> environment = const {},
}) => throw const AcpFailure('A page cannot start a program');
