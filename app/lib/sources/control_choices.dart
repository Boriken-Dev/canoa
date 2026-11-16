import 'dart:async';
import 'dart:convert';

import '../accounts/account_models.dart';
import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import 'source_choices.dart';
import 'sources.dart';

/// What the host lists of [source] before a session starts, asked over its control channel as [account] when one is named
/// (the machine's own login when null or [machineLogin]). The host runs the lookup, so a page that only reaches a host asks it
/// as the desktop's own app does. Null when the host cannot answer: one without the method, a link that is down, or a source it
/// does not run, so the caller can ask another way.
Future<SourceChoices?> choicesOverControl(HostControl control, AgentSource source, {String? account}) async {
  final entry = source.hostEntry;
  if (entry == null) return null;
  try {
    final client = await control.ready(wait: false);
    final result = await client.call(
      'options/choices',
      {'entry': entry, if (account != null && account != machineLogin) 'account': account},
      null,
      // A program that lists only in its own session is started once to be asked.
      const Duration(minutes: 2),
    );
    return result is Map ? SourceChoices.parse(jsonEncode(result)) : null;
  } on ControlFailed {
    return null;
  } on Object {
    return null;
  }
}
