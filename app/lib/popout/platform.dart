import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;

import 'platform_stub.dart'
    if (dart.library.io) 'platform_io.dart'
    if (dart.library.js_interop) 'platform_web.dart' as platform;
import 'popout_window.dart';
import 'popout_launch.dart';

/// How this platform opens a window for a conversation; [PopoutWindows.available] says whether it can.
PopoutWindows createPopoutWindows() => platform.createPopoutWindows();

/// The launch this process was started with when it is a pop-out window, from the program's arguments (a desktop engine's) or its
/// address (a page's); null for the app itself.
PopoutLaunch? popoutLaunch(List<String> args) => platform.popoutLaunch(args);

/// The window's end of the link to the window that holds the conversation.
Future<ChatViewLink> connectPopout(PopoutLaunch launch) => platform.connectPopout(launch);

/// Gets the window ready (its title) and shows it.
Future<void> preparePopoutWindow(PopoutLaunch launch) => platform.preparePopoutWindow(launch);

/// Closes this window, which is a pop-out window.
Future<void> closePopoutWindow() => platform.closePopoutWindow();
