import 'host_tool.dart';

HostTool? findHostTool() => null;

Future<int> freeLoopbackPort() => throw UnsupportedError('A page cannot listen');

Future<T> withFileBeside<T>(String path, String text, Future<T> Function(String file) use) =>
    throw UnsupportedError('A page cannot write files');
