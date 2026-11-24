import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;

import 'flutter_windows.dart';
import 'popout_launch.dart';
import 'popout_window.dart';

// A desktop's windows are windows of its own engine, which need no launch, link or window of their own to prepare.

PopoutWindows createPopoutWindows() => FlutterWindows();

PopoutLaunch? popoutLaunch(List<String> args) => null;

Future<ChatViewLink> connectPopout(PopoutLaunch launch) => throw UnsupportedError('A desktop window draws its conversation itself');

Future<void> preparePopoutWindow(PopoutLaunch launch) async {}

Future<void> closePopoutWindow() async {}
