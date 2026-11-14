import 'package:web/web.dart' as web;

void navigateTo(Uri address) => web.window.location.assign(address.toString());
