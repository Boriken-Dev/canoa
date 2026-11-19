import 'dart:convert';
import 'dart:io';

import 'package:canoa/theme/look.dart';
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

/// The families the vendor crates carry, by the ids and labels of their documents.
const claudeLook = LookId('claude', 'Claude');
const chatgptLook = LookId('chatgpt', 'ChatGPT');

/// Every family a host listing the crates' documents offers: the built-in ones, then the crates'.
const allLooks = [...LookId.builtIn, claudeLook, chatgptLook];
