import 'open_in.dart';

/// A page cannot start programs on the machine that served it: no system, so no action.
OpenInSystem systemOpenIn() => const _NoSystem();

final class _NoSystem implements OpenInSystem {
  const _NoSystem();

  @override
  HostOs? get os => null;
  @override
  Map<String, String> get environment => const {};
  @override
  bool isDirectory(String path) => false;
  @override
  bool isFile(String path) => false;
  @override
  String? registryDefault(String key) => null;
  @override
  String? bundlePath(String id) => null;
  @override
  Future<void> start(String executable, List<String> arguments) =>
      Future.error(OpenInFailure('A page cannot open programs on the computer it is served from.'));
}
