#!/usr/bin/env bash
# The agent side of e2e/noise.mjs, in the place the bridge and nginx run: a Noise bridge shown a PIN and serving what it pairs,
# and on request a WebCrypto bridge allowing one key, each behind nginx ending TLS when NGINX is given.
#
#   bash noise-side.sh BRIDGE AGENT [NGINX]
#
# It prints one JSON object per line: the Noise side first, the WebCrypto side after a line "allow <peer id>" on its input. When
# its input ends it stops what it started, by process id, and removes its directory.
set -euo pipefail
bridge=$1
agent=$2
nginx=${3:-}
work=$(mktemp -d "${TMPDIR:-/tmp}/canoa-noise-side.XXXXXX")
pids=()
finish() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  rm -rf "$work"
}
trap finish EXIT
free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'; }
export COQUI_AGENTIC_PEER_HOME=$work/home

pair_port=$(free_port)
mkfifo "$work/pair.out"
"$bridge" pair-accept --listen "127.0.0.1:$pair_port" --method noise --timeout-seconds 600 > "$work/pair.out" 2> "$work/pair.err" &
pids+=($!)
exec 3< "$work/pair.out"
read -r pin <&3
read -r noise_peer <&3

noise_port=$(free_port)
"$bridge" serve --paired --method noise --listen "127.0.0.1:$noise_port" -- "$agent" > "$work/noise.out" 2> "$work/noise.err" &
pids+=($!)

scheme=ws
front_pair=127.0.0.1:$pair_port
front_noise=127.0.0.1:$noise_port
spki=
webcrypto_port=$(free_port)
if [ -n "$nginx" ]; then
  front=$(free_port)
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 -subj /CN=localhost \
    -addext subjectAltName=IP:127.0.0.1,DNS:localhost -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
  spki=$(openssl x509 -in "$work/cert.pem" -pubkey -noout | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary | base64)
  mkdir -p "$work/nginx/logs"
  cat > "$work/nginx/nginx.conf" <<CONF
daemon off;
worker_processes 1;
pid $work/nginx/nginx.pid;
error_log $work/nginx/logs/error.log;
events { worker_connections 64; }
http {
  access_log $work/nginx/logs/access.log;
  client_body_temp_path $work/nginx/body;
  proxy_temp_path $work/nginx/proxy;
  fastcgi_temp_path $work/nginx/fastcgi;
  uwsgi_temp_path $work/nginx/uwsgi;
  scgi_temp_path $work/nginx/scgi;
  map \$http_upgrade \$connection_upgrade { default upgrade; '' close; }
  server {
    listen 127.0.0.1:$front ssl;
    ssl_certificate $work/cert.pem;
    ssl_certificate_key $work/key.pem;
    ssl_protocols TLSv1.3;
    proxy_http_version 1.1;
    proxy_set_header Upgrade \$http_upgrade;
    proxy_set_header Connection \$connection_upgrade;
    proxy_read_timeout 300s;
    location /pair { proxy_pass http://127.0.0.1:$pair_port/; }
    location /noise { proxy_pass http://127.0.0.1:$noise_port/; }
    location /webcrypto { proxy_pass http://127.0.0.1:$webcrypto_port/; }
  }
}
CONF
  "$nginx" -p "$work/nginx" -c "$work/nginx/nginx.conf" &
  pids+=($!)
  scheme=wss
  front_pair=127.0.0.1:$front/pair
  front_noise=127.0.0.1:$front/noise
fi
for port in $pair_port $noise_port ${front:-}; do
  for _ in $(seq 100); do (exec 9<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null && break; sleep 0.1; done
done
printf '{"pin":"%s","pairUrl":"%s://%s","noiseUrl":"%s://%s","noisePeer":"%s","spki":"%s"}\n' \
  "$pin" "$scheme" "$front_pair" "$scheme" "$front_noise" "$noise_peer" "$spki"

while read -r command peer; do
  [ "$command" = allow ] || continue
  "$bridge" keygen "$work/webcrypto.pem" --method webcrypto > "$work/webcrypto.peer"
  "$bridge" serve --identity "$work/webcrypto.pem" --allow "$peer" --listen "127.0.0.1:$webcrypto_port" -- "$agent" \
    > /dev/null 2> "$work/webcrypto.err" &
  pids+=($!)
  for _ in $(seq 100); do (exec 9<>"/dev/tcp/127.0.0.1/$webcrypto_port") 2>/dev/null && break; sleep 0.1; done
  if [ -n "$nginx" ]; then url=wss://127.0.0.1:$front/webcrypto; else url=ws://127.0.0.1:$webcrypto_port; fi
  printf '{"webcryptoUrl":"%s","webcryptoPeer":"%s"}\n' "$url" "$(cat "$work/webcrypto.peer")"
done
# What reached nginx, for the run to check every connection was upgraded through it.
if [ -n "$nginx" ]; then
  printf '{"access":%s}\n' "$(python3 -c 'import json,sys; print(json.dumps(open(sys.argv[1]).read()))' "$work/nginx/logs/access.log")"
fi
grep -h -E 'proving by|pairing by|paired with|refused' "$work"/*.err >&2 || true
