// The profile items that ship with the host, written with the Canoa SDK: who the session's programs say they are (`identity`) and how
// they reach the network (`network`). The routes are the host's; this item only names which of them its sessions leave by. The module is
// the library: its id and version are its package's, its providers the classes it exports.
import { EnvProfileItem, NetworkProfileItem } from './sdk/profile.js';

/** The name and address git commits under, the account label the session reports, and any other variable that says who is working. */
export class Identity extends EnvProfileItem {
  static item = 'identity';
  static title = 'Identity';
  static help =
    'The name and address git commits under, the account label the session reports, and any other variable that says who is working.';
  static values = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "properties": {
      "name": {
        "type": [
          "string",
          "null"
        ],
        "title": "Name",
        "description": "The name git commits under."
      },
      "email": {
        "type": [
          "string",
          "null"
        ],
        "title": "Email address",
        "description": "The address git commits under."
      },
      "account": {
        "type": [
          "string",
          "null"
        ],
        "title": "Account label",
        "description": "What AGENTS_ACCOUNT says to the session's programs."
      },
      "variables": {
        "type": "array",
        "items": {
          "type": "string"
        },
        "title": "Other variables",
        "description": "One NAME=value per line.",
        "default": []
      }
    },
    "additionalProperties": false
  };
  static variables = {
    name: ['GIT_AUTHOR_NAME', 'GIT_COMMITTER_NAME'],
    email: ['GIT_AUTHOR_EMAIL', 'GIT_COMMITTER_EMAIL'],
    account: ['AGENTS_ACCOUNT'],
  };
  static lines = 'variables';
  static lineLabel = 'other variable';
  static note = 'tells its programs who is working';
}

/** How the session's programs reach the network. */
export class Network extends NetworkProfileItem {
  static item = 'network';
  static title = 'Network';
  static help =
    "The routes this session's programs leave by. A route is one of the host's (Settings, Network, Proxy): the system's own, direct, a proxy, a PAC script or a program, each set once on the host. With none named, the host's own route carries them.";
  static values = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "properties": {
      "routes": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "route": {
              "type": "string",
              "title": "Route",
              "description": "The name of a route the host has. A route the host lacks, or one that does not open, refuses the session."
            },
            "hosts": {
              "type": "array",
              "items": {
                "type": "string"
              },
              "title": "Destinations",
              "description": "The destinations this route is for: a host name, *.name for a name and what is under it, an address, or a regular expression matched against the whole host. Empty: every destination no other route names.",
              "default": []
            }
          },
          "additionalProperties": false,
          "required": [
            "route"
          ]
        },
        "title": "Routes",
        "description": "In order: the first route with destinations that matches decides; then the route of the account the session runs on for the account's own addresses; then the route with no destinations; then the host's route.",
        "default": []
      },
      "bypass": {
        "type": "array",
        "items": {
          "type": "string"
        },
        "title": "Hosts that go direct",
        "description": "In the NO_PROXY form. Empty keeps what the routes choose.",
        "default": []
      },
      "client_certificate_hosts": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "pattern": {
              "type": "string",
              "title": "Hosts",
              "description": "A host name (`api.example.com`), a name and everything under it (`*.example.com`), an address, or any other regular expression, matched whole and without regard to case against the destination as the program names it (a name, or an address): `.*` matches every destination, `(api|auth)[.]example[.]com` two."
            },
            "identity": {
              "type": "string",
              "title": "Certificate",
              "description": "`system` presents what the operating system's keystores choose by the authorities the server names, as a browser would; or the thumbprint (SHA-256, in hex) of one certificate the host lists as available.",
              "default": "system"
            }
          },
          "additionalProperties": false,
          "required": [
            "pattern"
          ]
        },
        "title": "Hosts that get a client certificate",
        "description": "Each entry is a pattern for hosts (a host name, *.name for a name and what is under it, an address, or any regular expression matched against the whole host: .* is every host) and the certificate to present to them: system (what your keystores choose by the authorities the server names, as a browser would) or one of the certificates this computer lists. The first entry that matches a host decides. For these hosts alone the host's proxy for the session ends the program's TLS with a certificate made on demand by an authority this host process makes in memory (never written to disk; limited to exactly these hosts when every pattern is a plain host, *.name or address) that the session's programs alone are told to trust, and makes its own TLS to the host, presenting the certificate. A program that pins the certificates it trusts, or reads no CA file variable (Windows' own tools), fails for them. Empty ends no TLS.",
        "default": []
      }
    },
    "additionalProperties": false
  };
  static form = {
    "fields": {
      "client_certificate_hosts": {
        "form": {
          "fields": {
            "pattern": {
              "placeholder": ".*"
            },
            "identity": {
              "widget": "client_certificate"
            }
          }
        }
      }
    }
  };
}
