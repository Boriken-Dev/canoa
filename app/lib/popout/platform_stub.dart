import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/widgets.dart' show Widget, runApp;

import 'popout_window.dart';
import 'popout_launch.dart';

PopoutWindows createPopoutWindows() => NoPopoutWindows();
PopoutLaunch? popoutLaunch(List<String> args) => null;
Future<ChatViewLink> connectPopout(PopoutLaunch launch) => throw UnsupportedError('No pop-out windows here');
Future<void> preparePopoutWindow(PopoutLaunch launch) async {}
Future<void> closePopoutWindow() async {}

void startApp(PopoutWindows windows, Widget app) => runApp(windows.host(app));
