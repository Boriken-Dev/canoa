import 'dart:convert';
import 'dart:io';

import 'package:canoa/theme/vendor_looks.dart';

/// What `looks/list` answers: the documents the vendor crates carry, read from the repository.
Map<String, Object?> crateLooks() => {
  'looks': [
    for (final path in ['../crates/canoa-claude/looks/claude.json', '../crates/canoa-codex/looks/chatgpt.json'])
      jsonDecode(File(path).readAsStringSync()),
  ],
};

/// Gives the app the families a host would list, as an app connected to one has them.
void listCrateLooks() => vendorLooks.adopt(crateLooks());
