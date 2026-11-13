/// A page cannot start a program.
const bool canRunLocalAgents = false;

Future<String?> findProgram(String program, {List<String> searchDirs = const []}) async => null;

String? environmentVariable(String name) => null;

Future<List<int>> readLocalFile(String path) => throw UnsupportedError('A page cannot read a file by its path');

bool folderExists(String folder) => true;

bool inGitRepository(String folder) => false;

Future<int?> localFileSize(String path) async => null;

/// A file or folder inside a session's folder, as [resolveInside] answers.
typedef InsideFile = ({String path, bool folder, int size});

Future<List<String>> listFolderEntries(String folder) async => const [];

InsideFile? resolveInside(String folder, String relative) => null;
