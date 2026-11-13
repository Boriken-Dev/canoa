import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;

import 'popout_window.dart';
import 'popout_launch.dart';

PopoutWindows createPopoutWindows() => const NoPopoutWindows();
PopoutLaunch? popoutLaunch(List<String> args) => null;
Future<ChatViewLink> connectPopout(PopoutLaunch launch) => throw UnsupportedError('No pop-out windows here');
Future<void> preparePopoutWindow(PopoutLaunch launch) async {}
Future<void> closePopoutWindow() async {}
