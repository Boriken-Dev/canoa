import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:flutter/widgets.dart' show Widget;

import 'platform_stub.dart'
    if (dart.library.io) 'platform_io.dart'
    if (dart.library.js_interop) 'platform_web.dart' as platform;
import 'popout_window.dart';
import 'popout_launch.dart';

/// How this platform opens a window for a conversation; [PopoutWindows.available] says whether it can.
PopoutWindows createPopoutWindows() => platform.createPopoutWindows();

/// Runs [app] with the place [windows] draws its windows of this engine: the app's root goes through it. The browser's engine may have
/// started with no view of its own, to give a window a view each of the same app, and the page's is then drawn here too.
void startApp(PopoutWindows windows, Widget app) => platform.startApp(windows, app);

/// The launch this process was started with when it is a pop-out page, from its address; null for the app itself and for a desktop,
/// whose windows are of its own engine.
PopoutLaunch? popoutLaunch(List<String> args) => platform.popoutLaunch(args);

/// The page's end of the link to the page that holds the conversation.
Future<ChatViewLink> connectPopout(PopoutLaunch launch) => platform.connectPopout(launch);

/// Gets the page ready (its title).
Future<void> preparePopoutWindow(PopoutLaunch launch) => platform.preparePopoutWindow(launch);

/// Closes this page, which is a pop-out window.
Future<void> closePopoutWindow() => platform.closePopoutWindow();
