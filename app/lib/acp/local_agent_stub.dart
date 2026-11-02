import 'agent_commands.dart';
import 'client.dart';

/// A page cannot start a program.
const bool canRunLocalAgents = false;

Future<bool> isInstalled(AgentCommand command) async => false;

Future<VerifiedAgent> startLocalAgent(
  AgentCommand command, {
  AcpDecider? decider,
  String? workingDirectory,
}) => throw const AcpFailure('A page cannot start a program');
