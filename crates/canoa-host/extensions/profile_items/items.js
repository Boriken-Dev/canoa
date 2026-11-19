// The profile items that ship with the host, written with the Canoa SDK: who the session's programs say they are (`identity`) and how
// they reach the network (`proxy`). The proxy itself is the host's; this item only says which network its values ask for. The module is
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
export class Proxy extends NetworkProfileItem {
  static item = 'proxy';
  static title = 'Proxy';
  static help =
    "How this session's programs reach the network. By default the host runs a proxy (shared with the sessions that have the same settings) which sends each connection as this computer's own network would.";
  static values = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "properties": {
      "mode": {
        "type": "string",
        "enum": [
          "system",
          "none",
          "proxy",
          "pac",
          "command"
        ],
        "x-enum-labels": {
          "system": "Through a proxy the host runs on this computer's loopback address, which follows the system.",
          "none": "No proxy is given to the session's programs.",
          "proxy": "The proxy at the address.",
          "pac": "As system, choosing by the PAC script.",
          "command": "A program of the person's that listens and is given to the session's programs."
        },
        "title": "Reach the network",
        "description": "system: through a proxy the host runs on this computer's loopback address, which follows the system (PAC, login, bypass list). none: no proxy is given to its programs. proxy: the proxy at the address below. pac: as system, choosing by the PAC script below. command: a program of yours that listens and is given to its programs.",
        "default": "system"
      },
      "url": {
        "type": [
          "string",
          "null"
        ],
        "title": "Address",
        "description": "proxy: [scheme://][user[:password]@]host[:port]. pac: the script's address or a file."
      },
      "command": {
        "type": "array",
        "items": {
          "type": "string"
        },
        "title": "Program and arguments",
        "description": "command: the program first, then its arguments; {listen} is replaced by the address host:port it must listen on.",
        "default": []
      },
      "bypass": {
        "type": "array",
        "items": {
          "type": "string"
        },
        "title": "Hosts that go direct",
        "description": "In the NO_PROXY form. Empty keeps what the network chooses.",
        "default": []
      },
      "user": {
        "type": [
          "string",
          "null"
        ],
        "title": "Proxy user name",
        "description": "The user the proxy asks for."
      },
      "password_variable": {
        "type": [
          "string",
          "null"
        ],
        "title": "Variable holding the password",
        "description": "The name of an environment variable of the host that holds the proxy user's password; the password is never kept in the profile."
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
        "description": "system and pac only. Each entry is a pattern for hosts (a host name, *.name for a name and what is under it, an address, or any regular expression matched against the whole host: .* is every host) and the certificate to present to them: system (what your keystores choose by the authorities the server names, as a browser would) or one of the certificates this computer lists. The first entry that matches a host decides. For these hosts alone the proxy ends the program's TLS with a certificate made on demand by an authority this host process makes in memory (never written to disk; limited to exactly these hosts when every pattern is a plain host, *.name or address) that the session's programs alone are told to trust, and makes its own TLS to the host, presenting the certificate. A program that pins the certificates it trusts, or reads no CA file variable (Windows' own tools), fails for them. Empty ends no TLS.",
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
