import 'package:canoa/sources/source_choices.dart';
import 'package:flutter_test/flutter_test.dart';

ModelChoice model(String id, [List<String> capabilities = const []]) =>
    ModelChoice(id: id, name: id, capabilities: capabilities);

void main() {
  final models = [
    model('gpt-5-mini', ['chat', 'tool_use']),
    model('text-embedding-3-small', ['embed']),
    model('whisper-1', ['speech_to_text']),
    model('some-new-model'),
  ];

  test('a picker for a use lists the models that report it, then the ones that report nothing, and never one that reports another use', () {
    final chat = modelsFor(models, 'chat');
    expect([for (final each in chat.offered) each.id], ['gpt-5-mini']);
    expect([for (final each in chat.unreported) each.id], ['some-new-model']);
    expect(
      [for (final each in chat.listed) each.id],
      ['gpt-5-mini', 'some-new-model'],
    );
    expect(
      [for (final each in modelsFor(models, 'embed').listed) each.id],
      ['text-embedding-3-small', 'some-new-model'],
    );
    expect(
      [for (final each in modelsFor(models, 'speech_to_text').offered) each.id],
      ['whisper-1'],
    );
  });

  test('an agent session is offered the models that can chat: an embedding model of the same account is not one', () {
    final choices = SourceChoices(
      known: true,
      models: [
        model('chat-1', ['chat']),
        model('embed-1', ['embed']),
        model('unknown-1'),
      ],
    );
    final option = choices.options().firstWhere((each) => each.id == 'model');
    expect(
      [for (final each in option.choices) each.value],
      ['chat-1', 'unknown-1'],
    );
  });
}
