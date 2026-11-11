import 'dart:io';

import 'permission_rules.dart' show RuleLog;

/// `permissions-log.jsonl` beside canoa.cfg at [configPath], which only grows; null when there is no such place.
RuleLog? platformRuleLog({String? configPath}) =>
    configPath == null ? null : FileRuleLog('${File(configPath).parent.path}${Platform.pathSeparator}permissions-log.jsonl');

/// A file each line is appended to, never rewritten.
final class FileRuleLog implements RuleLog {
  FileRuleLog(this.path);
  final String path;

  @override
  String get location => path;

  @override
  Future<void> append(String line) async {
    final file = File(path);
    await file.parent.create(recursive: true);
    await file.writeAsString('$line\n', mode: FileMode.append, flush: true);
  }
}
