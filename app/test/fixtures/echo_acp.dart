// A small ACP agent on its standard streams, for the app's tests. A prompt asks for a tool's permission first, and answers with what
// the person said. With --stubborn it ignores its input closing, like an agent that will not leave.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

void main(List<String> args) async {
  final stubborn = args.contains('--stubborn');
  for (final arg in args) {
    if (arg.startsWith('--pid-file=')) File(arg.substring(11)).writeAsStringSync('$pid');
  }
  var nextId = 1000;
  final waiting = <int, void Function(Map<String, dynamic>)>{};
  void send(Map<String, dynamic> message) => stdout.writeln(jsonEncode({'jsonrpc': '2.0', ...message}));
  await for (final line in stdin.transform(utf8.decoder).transform(const LineSplitter())) {
    final message = jsonDecode(line) as Map<String, dynamic>;
    final id = message['id'];
    final method = message['method'];
    if (method == null) {
      waiting.remove(id)?.call(message);
    } else if (method == 'initialize') {
      send({'id': id, 'result': {'protocolVersion': 1, 'agentCapabilities': {}}});
    } else if (method == 'session/new') {
      send({'id': id, 'result': {'sessionId': 'local-session'}});
    } else if (method == 'session/prompt') {
      final asking = nextId++;
      waiting[asking] = (answer) {
        final chose = (answer['result'] as Map)['outcome'];
        send({'method': 'session/update', 'params': {'sessionId': 'local-session', 'update': {
          'sessionUpdate': 'agent_message_chunk',
          'content': {'type': 'text', 'text': 'the person said ${jsonEncode(chose)}'},
        }}});
        send({'id': id, 'result': {'stopReason': 'end_turn'}});
      };
      send({'id': asking, 'method': 'session/request_permission', 'params': {
        'sessionId': 'local-session',
        'toolCall': {'toolCallId': 't1', 'title': 'Run a command', 'kind': 'execute'},
        'options': [
          {'optionId': 'yes', 'name': 'Yes', 'kind': 'allow_once'},
          {'optionId': 'no', 'name': 'No', 'kind': 'reject_once'},
        ],
      }});
    } else if (id != null) {
      send({'id': id, 'result': <String, dynamic>{}});
    }
  }
  if (stubborn) await Future<void>.delayed(const Duration(hours: 1));
  exit(0);
}
