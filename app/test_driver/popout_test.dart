// Drives `integration_test/popout_test.dart`: what it asks the app to do and what it expects the app to say. Plain Dart: a failed
// check is collected, and the exit code says how it went.
import 'dart:convert';
import 'dart:io';

// ignore: depend_on_referenced_packages
import 'package:flutter_driver/flutter_driver.dart';

final _failures = <String>[];

void check(bool ok, String what) {
  stdout.writeln('${ok ? 'ok     ' : 'FAILED '}$what');
  if (!ok) _failures.add(what);
}

Future<void> main() async {
  final driver = await FlutterDriver.connect();
  Future<Map<String, dynamic>> ask(String message) async {
    final said = jsonDecode(await driver.requestData(message, timeout: const Duration(seconds: 90))) as Map<String, dynamic>;
    if (said['error'] != null) throw StateError('$message: ${said['error']}\n${said['stack']}');
    return said;
  }

  try {
    var s = await ask('state');
    check((s['windows'] as List).isEmpty && s['chats'] == 0, 'nothing is popped out to begin with');
    check(s['pending'].toString() == '[Edit a.txt, Edit b.txt]' && s['status'] == 'working', 'the turn runs and two requests wait');
    check(s['visible'] == true, 'the main window is shown');

    s = await ask('open');
    final window = (s['windows'] as List).single as Map<String, dynamic>;
    check(window['link'] == false, 'the window is of the engine: it has no link');
    check(window['title'] == 'Fix calc' && (window['width'] as num) > 0 && (window['height'] as num) > 0, 'it has a title and the size of a real window: $window');
    check(s['chats'] == 1, 'the window draws the conversation');
    check((s['clients'] as List).contains(window['id']), 'it is attached as a client of the conversation itself');
    final reading = s['said']['Reading calc.js.'] as Map;
    check((reading['window'] as int) > 0 && (reading['main'] as int) > 0, 'the reply is drawn in the window and in the main window: $reading');

    s = await ask('answer_in_window');
    check(s['answers'].toString() == '[5]', 'a request answered in the window reaches the agent once');
    check(s['settled'].toString() == '[${window['id']}]', 'the conversation says the window answered it');
    check(s['pending'].toString() == '[Edit b.txt]', 'the second request still waits');
    final edit = s['said']['Edit b.txt'] as Map;
    check((edit['window'] as int) > 0 && (edit['main'] as int) > 0, 'it waits in both windows: $edit');

    s = await ask('answer_in_main');
    check(s['answers'].toString() == '[5, 6]' && s['pending'].toString() == '[]', 'a request answered in the main window reaches the agent once, and the window shows nothing waiting');
    check(s['settled'].toString() == '[${window['id']}, main]', 'the conversation says who answered each: ${s['settled']}');
    final said = s['said']['Answered in Canoa.'] as Map;
    check((said['window'] as int) > 0, 'the window says it was answered in the main window: $said');

    s = await ask('both_send');
    final queue = s['queue'] as List;
    check(queue.length == 2 && queue.contains('from the window') && queue.contains('from the main window'), 'a message from each window is in the one queue: $queue');
    check(s['prompts'] == 1, 'only the first message went to the agent');
    final waiting = s['said']['2 messages wait for this turn to end.'] as Map;
    check((waiting['window'] as int) > 0, 'the window shows both waiting: $waiting');

    s = await ask('reply');
    final fixed = s['said']['And it is fixed.'] as Map;
    check((fixed['window'] as int) > 0 && (fixed['main'] as int) > 0, 'a reply reaches both windows at once: $fixed');
    check(s['pending'].toString() == '[Edit c.txt]', 'a request that arrives waits in both windows: ${s['pending']}');
    final third = s['said']['Edit c.txt'] as Map;
    check((third['window'] as int) > 0 && (third['main'] as int) > 0, 'it is drawn in both: $third');

    s = await ask('close_popout');
    check((s['windows'] as List).isEmpty && s['chats'] == 0, 'closing the pop-out closes it');
    check(s['visible'] == true, 'the main window is not closed with it');
    check(s['status'] == 'working' && s['pending'].toString() == '[Edit c.txt]', 'the turn and the request that waits are as they were: ${s['status']} ${s['pending']}');
    check(!(s['clients'] as List).contains(window['id']), 'the window is no longer a client of the conversation');

    s = await ask('open');
    check((s['windows'] as List).length == 1 && s['chats'] == 1, 'it opens again');
    s = await ask('close_main');
    if (s['tray'] == true) {
      check(s['visible'] == false, 'closing the main window puts it away in the tray');
    } else {
      check(s['visible'] == true, 'with no tray, closing the main window does not hide it where nothing brings it back');
    }
    check((s['windows'] as List).length == 1 && s['chats'] == 1, 'the pop-out is still open');
    check(s['status'] == 'working', 'the turn goes on');
    s = await ask('restore');
    check(s['visible'] == true, 'the main window is back');

    s = await ask('dock');
    check((s['windows'] as List).isEmpty && s['pending'].toString() == '[Edit c.txt]', 'docked back from the window\'s own button, the conversation is as it was');
  } finally {
    await driver.close();
  }
  if (_failures.isNotEmpty) {
    stderr.writeln('${_failures.length} check(s) failed');
    exit(1);
  }
  stdout.writeln('all checks passed');
}
