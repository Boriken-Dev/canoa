// Settings' Network page over a scripted host: the routes with the default badged and where each is used, a route made from the schema
// of its provider as one `routes/set` call, the default changed, a route tried and removed after asking, and the host's refusal shown
// in its own words.
@Timeout(Duration(minutes: 3))
library;

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/settings/network_routes.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/settings_fixture.dart';

/// A host with the two routes every host has, one the person made, and the manual provider to make more.
final class ScriptedRoutes {
  ScriptedRoutes({this.refuse});

  String? refuse;
  final calls = <(String, Map<String, Object?>)>[];
  String defaultRoute = 'system';
  final made = <String, Map<String, Object?>>{
    'corp': {'https_proxy': 'proxy.corp:8080'},
  };

  static final _manual = {
    'type': 'object',
    'properties': {
      'all_proxy': {'type': 'string', 'title': 'Any other proxy'},
      'password_env': {'type': 'string', 'title': 'Password variable'},
    },
  };

  Future<Object?> call(String method, Map<String, Object?> params) async {
    calls.add((method, params));
    switch (method) {
      case 'routes/list':
        return {
          'default': defaultRoute,
          'routes': [
            for (final name in ['system', 'direct', ...made.keys])
              {
                'name': name,
                'provider': made.containsKey(name) ? 'manual' : name,
                'title': made.containsKey(name) ? 'Manual proxy' : (name == 'system' ? 'System' : 'Direct'),
                'default': name == defaultRoute,
                'builtin': !made.containsKey(name),
                'settings': made[name],
                'uses': {'host': name == defaultRoute, 'accounts': name == 'corp' ? ['work'] : [], 'profiles': []},
                'leases': name == defaultRoute ? 1 : 0,
                'state': name == defaultRoute ? 'following the system' : null,
                'problems': name == 'corp' ? ['password_env names PW, a variable that is not set'] : [],
              },
          ],
        };
      case 'routes/offers':
        return {
          'offers': [
            {'provider': 'manual', 'origin': 'built-in', 'title': 'Manual proxy', 'description': 'The proxy you type.', 'sharing': 'Shared', 'secrets': ['password_env'], 'schema': _manual, 'form': {}},
          ],
        };
      case 'routes/set':
        if (refuse != null) throw ControlFailed(-32000, refuse!);
        made['${params['name']}'] = {...(params['settings']! as Map).cast<String, Object?>()};
        return {'route': {'name': params['name']}, 'restart': true};
      case 'routes/default':
        defaultRoute = '${params['name']}';
        return {'default': defaultRoute, 'restart': true};
      case 'routes/remove':
        made.remove('${params['name']}');
        return {'removed': true, 'restart': true};
      case 'routes/test':
        return {'ok': true, 'url': 'https://tools.example/', 'via': ['http://proxy.corp:8080'], 'direct': false, 'millis': 12};
    }
    throw ControlFailed(-32601, 'no method $method');
  }

  List<Map<String, Object?>> called(String method) => [for (final (name, params) in calls) if (name == method) params];
}

void main() {
  Future<ScriptedRoutes> open(WidgetTester tester, {ScriptedRoutes? scripted}) async {
    final host = scripted ?? ScriptedRoutes();
    await openSettings(tester, section: 'network', size: const Size(1100, 2000), networkRoutes: NetworkRoutes(host.call));
    await tester.pumpAndSettle();
    return host;
  }

  testWidgets('lists the routes, the host\'s badged, with where each is used and what is wrong with it', (tester) async {
    await open(tester);
    expect(find.byKey(const Key('route system')), findsOneWidget);
    expect(find.byKey(const Key('route direct')), findsOneWidget);
    expect(find.byKey(const Key('route corp')), findsOneWidget);
    expect(find.byKey(const Key('route-default system')), findsOneWidget);
    expect(find.byKey(const Key('route-default corp')), findsNothing);
    expect(find.textContaining('Accounts: work'), findsOneWidget);
    expect(find.byKey(const Key('route-issue corp')), findsOneWidget);
    // The routes every host has are neither edited nor removed.
    expect(find.byKey(const Key('route-edit system')), findsNothing);
    expect(find.byKey(const Key('route-remove direct')), findsNothing);
    expect(find.byKey(const Key('route-edit corp')), findsOneWidget);
  });

  testWidgets('a route is made from its provider\'s form as one call, and made the default', (tester) async {
    final host = await open(tester);
    await tester.tap(find.byKey(const Key('add-route')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('route-name')), 'socks');
    await tester.enterText(find.byKey(const Key('route-form-field-all_proxy')), 'socks5://proxy:1080');
    await tester.tap(find.byKey(const Key('route-save')));
    await tester.pumpAndSettle();
    expect(host.called('routes/set').single, {'name': 'socks', 'provider': 'manual', 'settings': {'all_proxy': 'socks5://proxy:1080'}});
    expect(find.byKey(const Key('route socks')), findsOneWidget);

    await tester.tap(find.byKey(const Key('route-make-default socks')));
    await tester.pumpAndSettle();
    expect(host.called('routes/default').single, {'name': 'socks'});
    expect(find.byKey(const Key('route-default socks')), findsOneWidget);
  });

  testWidgets('the host\'s refusal of a route is shown in its own words and nothing is made', (tester) async {
    final host = await open(tester, scripted: ScriptedRoutes(refuse: 'name a proxy: http_proxy, https_proxy or all_proxy'));
    await tester.tap(find.byKey(const Key('add-route')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('route-name')), 'empty');
    await tester.tap(find.byKey(const Key('route-save')));
    await tester.pumpAndSettle();
    expect(find.text('name a proxy: http_proxy, https_proxy or all_proxy'), findsOneWidget);
    expect(host.made.containsKey('empty'), isFalse);
  });

  testWidgets('a route is tried, and removed only after asking', (tester) async {
    final host = await open(tester);
    await tester.tap(find.byKey(const Key('route-test-run corp')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('route-test corp')), findsOneWidget);
    expect(find.textContaining('http://proxy.corp:8080'), findsOneWidget);

    await tester.tap(find.byKey(const Key('route-remove corp')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('remove-route')), findsOneWidget);
    expect(find.textContaining('Accounts: work'), findsWidgets, reason: 'the dialog names what uses it');
    await tester.tap(find.text('Keep it'));
    await tester.pumpAndSettle();
    expect(host.called('routes/remove'), isEmpty);
    await tester.tap(find.byKey(const Key('route-remove corp')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('remove-route-confirm')));
    await tester.pumpAndSettle();
    expect(host.called('routes/remove').single, {'name': 'corp'});
    expect(find.byKey(const Key('route corp')), findsNothing);
  });
}
