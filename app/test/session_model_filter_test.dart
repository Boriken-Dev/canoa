// A session chats, so the model picker of a session leaves out a model its source says does something else, keeps the ones that chat
// and the ones that say nothing, and a name no listing holds.
import 'package:canoa/chat/session_picker.dart' show offeredForSession;
import 'package:canoa/sources/source_choices.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a model that says it does not chat is not offered to start a session on', () {
    expect(offeredForSession(const ModelChoice(id: 'embed-3', name: 'embed-3', capabilities: ['embed'])), isFalse);
    expect(offeredForSession(const ModelChoice(id: 'whisper', name: 'whisper', capabilities: ['speech_to_text'])), isFalse);
  });

  test('a model that chats, one that says nothing and one the listing does not hold are offered', () {
    expect(offeredForSession(const ModelChoice(id: 'sonnet', name: 'Sonnet', capabilities: ['chat', 'tool_use'])), isTrue);
    expect(offeredForSession(const ModelChoice(id: 'mystery', name: 'mystery')), isTrue);
    expect(offeredForSession(null), isTrue);
  });
}
