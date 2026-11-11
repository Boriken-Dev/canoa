// What a fake host answers when the app asks `canoa options`: the shapes the real command prints, for Claude Code 2.1.318 (the
// recorded models, their resolved ids and efforts) and a Codex whose models take efforts of their own.
import 'dart:convert';

import 'package:canoa/sources/source_choices.dart';
import 'package:canoa/sources/sources.dart';

const _claudeEfforts = ['low', 'medium', 'high', 'xhigh', 'max'];

final Map<String, Object?> claudeAnswer = {
  'known': true,
  'program': 'claude',
  'program_version': '2.1.318 (Claude Code)',
  'models': [
    {'id': 'default', 'name': 'Default (Opus 5.5)', 'resolved': 'claude-opus-5-5', 'version': 'Opus 5.5', 'efforts': _claudeEfforts, 'is_default': true},
    {'id': 'opus', 'name': 'Opus 5.5', 'resolved': 'claude-opus-5-5', 'version': 'Opus 5.5', 'efforts': _claudeEfforts},
    {'id': 'fable', 'name': 'Fable 5.1', 'resolved': 'claude-fable-5-1', 'version': 'Fable 5.1', 'efforts': _claudeEfforts},
    {'id': 'sonnet', 'name': 'Sonnet 5.5', 'resolved': 'claude-sonnet-5-5', 'version': 'Sonnet 5.5', 'efforts': _claudeEfforts},
    {'id': 'haiku', 'name': 'Haiku 5.5', 'resolved': 'claude-haiku-5-5', 'version': 'Haiku 5.5', 'efforts': _claudeEfforts},
  ],
};

final Map<String, Object?> codexAnswer = {
  'known': true,
  'program': 'codex',
  'program_version': 'codex-cli 0.166.0',
  'models': [
    {'id': 'gpt-6-luna', 'name': 'GPT-6 Luna', 'efforts': ['low', 'medium', 'high'], 'default_effort': 'medium', 'is_default': true},
    {'id': 'gpt-6-sol', 'name': 'GPT-6 Sol', 'efforts': ['low', 'high', 'xhigh'], 'default_effort': 'high'},
  ],
};

/// The answers by source label; a source not here cannot say before it starts, as the host answers for a program entry.
SourceChoices? fixtureChoices(String label) {
  final answer = switch (label) {
    'Claude' => claudeAnswer,
    'Codex' => codexAnswer,
    _ => null,
  };
  return answer == null ? null : SourceChoices.parse(jsonEncode(answer));
}

/// A loader that answers [fixtureChoices] and counts what it was asked.
final class FakeHost {
  final asked = <String>[];

  Future<SourceChoices?> call(AgentSource source) async {
    asked.add(source.label);
    return fixtureChoices(source.label);
  }
}
