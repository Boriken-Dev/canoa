import 'package:web/web.dart' as web;

bool openPage(Uri address) {
  web.window.open(address.toString(), '_blank');
  return true;
}
