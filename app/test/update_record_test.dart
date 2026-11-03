// What the update recorder keeps, and which rendering each kind of update gets.
import 'dart:convert';

import 'package:canoa/acp/client.dart';
import 'package:canoa/chat/update_record.dart';
import 'package:flutter_test/flutter_test.dart';

AcpSessionUpdate _update(Map<String, Object?> raw) => AcpSessionUpdate('s1', raw);

void main() {
  test('a shape keeps the protocol words and nothing anyone wrote', () {
    final shape = jsonEncode(shapeOf({
      'sessionUpdate': 'tool_call',
      'toolCallId': 'call-secret-1',
      'title': 'Read the secret file',
      'kind': 'read',
      'status': 'Not A Status secret',
      'rawInput': {'file_path': '/home/secret.txt', 'limit': 20, 'whole': true, 'ratio': 0.5, 'nothing': null},
      'content': [
        {'type': 'content', 'content': {'type': 'text', 'text': 'secret one'}},
        {'type': 'content', 'content': {'type': 'text', 'text': 'secret two'}},
        {'type': 'diff', 'path': 'a', 'oldText': 'secret', 'newText': 'secret'},
      ],
      'secret key with spaces': 'x',
    }));
    expect(shape, isNot(contains('secret')));
    expect(shape, contains('"kind":"read"'));
    expect(shape, contains('"status":"string"'), reason: 'a word under a protocol key, but not one the protocol could use');
    expect(shape, contains('"limit":"int"'));
    expect(shape, contains('"whole":"bool"'));
    expect(shape, contains('"ratio":"double"'));
    expect(shape, contains('"nothing":"null"'));
    expect(shape, contains('"key":"string"'));
    final content = (jsonDecode(shape) as Map)['content'] as List;
    expect(content, hasLength(2), reason: 'each distinct element shape once');
  });

  test('a long list keeps at most a few distinct shapes, and a deep one ends', () {
    final many = [for (var i = 0; i < 50; i++) {'field$i': i}];
    expect(shapeOf(many) as List, hasLength(maxListShapes));
    Object? deep = 'bottom';
    for (var i = 0; i < 20; i++) {
      deep = {'next': deep};
    }
    expect(jsonEncode(shapeOf(deep)), contains('…'));
  });

  test('each kind has a rendering of its own, and anything else reaches the fallback', () {
    const text = {'type': 'text', 'text': 'hi'};
    final expected = {
      'user_message': Rendering.message,
      'user_message_chunk': Rendering.message,
      'agent_message': Rendering.message,
      'agent_message_chunk': Rendering.message,
      'agent_thought': Rendering.thought,
      'agent_thought_chunk': Rendering.thought,
      'tool_call': Rendering.toolCard,
      'tool_call_update': Rendering.toolCard,
      'terminal_update': Rendering.terminal,
      'plan': Rendering.plan,
      'available_commands_update': Rendering.commands,
      'usage_update': Rendering.usage,
      'config_option_update': Rendering.choices,
      'current_mode_update': Rendering.choices,
      'session_info_update': Rendering.title,
      'state_update': Rendering.status,
      'something_new': Rendering.unshown,
    };
    expected.forEach((kind, rendering) {
      expect(renderingOf(_update({'sessionUpdate': kind, 'content': text})), rendering, reason: kind);
    });
    expect(
      renderingOf(_update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'image', 'data': '', 'mimeType': 'image/png'}})),
      Rendering.unshown,
      reason: 'an image from the agent is not drawn',
    );
  });

  test('each distinct update is written once, and counted every time', () {
    final lines = <String>[];
    final recorder = UpdateRecorder(write: lines.add);
    for (final said in ['one', 'two', 'three']) {
      final update = _update({'sessionUpdate': 'agent_message_chunk', 'content': {'type': 'text', 'text': said}});
      recorder.record(update, renderingOf(update));
    }
    final odd = _update({'sessionUpdate': 'Something Odd', 'x': 1});
    recorder.record(odd, renderingOf(odd));
    expect(lines, hasLength(2));
    expect(lines.join(), isNot(contains('one')));
    expect(recorder.seen.first.count, 3);
    expect(recorder.unshown, {'other'}, reason: 'a kind that is not a protocol word is not kept either');
  });
}
