import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;

import 'platform_stub.dart'
    if (dart.library.io) 'platform_io.dart'
    if (dart.library.js_interop) 'platform_web.dart' as platform;
import 'popout_window.dart';
import 'popout_launch.dart';

/// How this platform opens a window for a conversation; [PopoutWindows.available] says whether it can.
PopoutWindows createPopoutWindows() => platform.createPopoutWindows();

/// The launch this process was started with when it is a pop-out page, from its address; null for the app itself and for a desktop,
/// whose windows are of its own engine.
PopoutLaunch? popoutLaunch(List<String> args) => platform.popoutLaunch(args);

/// The page's end of the link to the page that holds the conversation.
Future<ChatViewLink> connectPopout(PopoutLaunch launch) => platform.connectPopout(launch);

/// Gets the page ready (its title).
Future<void> preparePopoutWindow(PopoutLaunch launch) => platform.preparePopoutWindow(launch);

/// Closes this page, which is a pop-out window.
Future<void> closePopoutWindow() => platform.closePopoutWindow();
