import 'dart:io';

import 'update_record.dart';

/// A recorder writing to the file `CANOA_EVENT_LOG` names, appended to as each distinct update arrives, or null when it names none.
UpdateRecorder? updateRecorderFromEnvironment() {
  final path = Platform.environment['CANOA_EVENT_LOG'];
  if (path == null || path.isEmpty) return null;
  final file = File(path);
  return UpdateRecorder(write: (line) => file.writeAsStringSync('$line\n', mode: FileMode.append, flush: true));
}
