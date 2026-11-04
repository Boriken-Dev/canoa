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

Future<List<int>> readLocalFile(String path) => throw UnsupportedError('A page cannot read a file by its path');

bool inGitRepository(String folder) => false;

Future<int?> localFileSize(String path) async => null;
