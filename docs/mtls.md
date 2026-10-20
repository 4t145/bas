# Manually configured mutual TLS

`mtls://HOST:PORT` is a separate, explicitly authenticated transport. It is not
`tcp://`, and neither transport falls back to the other. Local same-user IPC
remains the default; plaintext TCP remains an explicit supported option.

The daemon requires a client certificate signed by its configured client CA.
Clients require the configured server CA and verify the certificate's DNS name
or IP SAN against the target. System DNS/hosts resolution is unchanged. There is
no insecure/skip-verification switch. An optional expected server name overrides
the verification name and DNS SNI, **not** the network destination.

## Trust boundary

Every trusted client accesses the **same execution environment and sessions** of
one daemon. Certificates do not create OS users, filesystem isolation, per-user
accounts, or per-certificate permissions. Only give client certificates to people
or machines you trust to use that environment.

Connected clients can edit the daemon's Agent configuration and install, remove,
check, authenticate and log out agents through RPC. Server paths are never opened
as client-local files. Terminal sign-in that requires launching a command locally
must still be completed on the server host. This is CA-based trust, not account
login, QR pairing, a device registry, a relay, or device revocation.

## Create certificates

Use your organization's CA if available. For a small manually managed deployment,
the following OpenSSL example creates a dedicated CA and short-lived leaf
certificates. Run it in a **new private directory outside source control** on a
trusted administrative machine. Replace `bas.example.internal` and `192.0.2.10`
with the actual service name/IP. Only include IP SANs if clients will use them.

```sh
umask 077
mkdir bas-certificates
cd bas-certificates

# Keep this encrypted CA key offline; never give it to the daemon or clients.
openssl req -x509 -newkey rsa:3072 -sha256 -days 365 \
  -keyout ca.key -out ca.pem -subj "/CN=Bas private CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign"

openssl req -new -newkey rsa:3072 -nodes \
  -keyout server.key -out server.csr -subj "/CN=Bas server"
printf '%s\n' \
  'basicConstraints=critical,CA:FALSE' \
  'keyUsage=critical,digitalSignature,keyEncipherment' \
  'extendedKeyUsage=serverAuth' \
  'subjectAltName=DNS:bas.example.internal,IP:192.0.2.10' > server.ext
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
  -days 30 -sha256 -extfile server.ext -out server.pem

# Issue a separate client certificate/key for each client.
openssl req -new -newkey rsa:3072 -nodes \
  -keyout client.key -out client.csr -subj "/CN=Bas client workstation"
printf '%s\n' \
  'basicConstraints=critical,CA:FALSE' \
  'keyUsage=critical,digitalSignature' \
  'extendedKeyUsage=clientAuth' > client.ext
openssl x509 -req -in client.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
  -days 30 -sha256 -extfile client.ext -out client.pem
```

Distribute files over an authenticated secure channel:

- Server: `server.pem`, `server.key`, and the CA bundle authorized to issue clients.
- Client: `client.pem`, `client.key`, and the CA bundle authorized to issue servers.
- Neither endpoint needs `ca.key`. Separate client/server CAs are also supported.

Keep private keys readable only by the intended service/client user (Unix mode
`0600` in a `0700` directory; equivalent restricted ACLs on Windows). The endpoint
keys in this example are unencrypted PEM for unattended loading; protect the files.
Certificate chains should be leaf-first PEM followed by intermediates. Paths are
file references, not PEM contents. Prefer absolute paths: relative paths resolve
against the process working directory, not the settings file.

## Start a daemon and CLI

```sh
bas --listen local --listen mtls://0.0.0.0:7338 \
  --tls-certificate /private/bas/server.pem \
  --tls-key /private/bas/server.key \
  --tls-client-ca /private/bas/ca.pem

bast --server mtls://bas.example.internal:7338 \
  --tls-ca /private/client/ca.pem \
  --tls-certificate /private/client/client.pem \
  --tls-key /private/client/client.key agents
```

To connect by a different address while verifying a known DNS certificate name,
add `--tls-server-name bas.example.internal` to `bast`. Do not use this to ignore
a name mismatch without independently confirming the expected server identity.

You may also add `--listen tcp://127.0.0.1` to the daemon. All listeners share
the same registry/runtime; they use separate tonic listener TLS configuration.
TLS flags apply only to `mtls://` listeners. Missing/incomplete TLS options fail
closed. TLS options on a non-TLS CLI target are rejected. No extra listeners are
implicitly opened when `--listen` is supplied.

Plaintext TCP is unauthenticated access to the same environment: exposing a plain
listener to untrusted peers bypasses the protection of the TLS listener. Choose
bind addresses and network access deliberately; Bas does not configure firewalls.

## Saved GUI connections

In Settings → Frontend → Service connections, select **mTLS** and enter the address,
server CA file, client certificate file, private key file, and optional expected
server name. These fields are paths only; never paste a private key. Save, then
explicitly select the profile to change management context or open its chat window.
BasG's `--server` override supplies an endpoint only; use a saved profile for mTLS.
Invalid saved connection configuration stops GUI startup rather than connecting
to a default local service. Correct the configuration (or deliberately supply a
different explicit `--server`); unreadable TLS files produce a connection error.

```json
{
  "frontend": {
    "selected": "work",
    "connections": [{
      "id": "work",
      "name": "Work server",
      "target": "mtls://bas.example.internal:7338",
      "tls": {
        "ca": "/private/client/ca.pem",
        "certificate": "/private/client/client.pem",
        "private_key": "/private/client/client.key"
      }
    }]
  }
}
```

The optional `tls.server_name` has the same meaning as the CLI flag. Existing
profiles without `tls` remain valid for IPC/plain TCP. All programs reading a
settings file containing these new fields must support this version.

Existing chat/authentication windows and service caches retain their own complete
connection options. Selecting another profile, including different credential
paths for the same address, does not borrow that profile's credentials midway
through an existing request.

## Rotation, reload, and revocation limits

- Daemon certificates, keys and client CA trust load at startup. Restart the daemon
  after changing them; Agent catalog hot reload does **not** reload TLS files.
- Each client channel loads credentials when constructed and retains them on
  reconnect. Some GUI operations create new channels and may read updated files,
  while existing chat channels retain the previous material. Restart BasG after
  rotation for a consistent identity; each new `bast` invocation loads files anew.
- Cache/window identity includes file references, not a content fingerprint.
  Overwriting a file does not automatically invalidate cached frontend state.
- There is no per-device revocation list, CRL/OCSP configuration, or certificate
  pin registry in this slice. A trusted CA authorizes every otherwise-valid client
  certificate it issues. Removing one leaf file on a server does not revoke it.
- To remove a compromised trust domain, replace the trusted client CA, reissue
  authorized clients and restart the daemon. Existing TLS sessions do not recheck
  certificate expiry/trust on each RPC; restarting terminates them. Short-lived
  certificates limit new handshakes after expiry but are not immediate revocation.

Tests generate temporary CA/key material at runtime using the published
MIT/Apache-2.0 `rcgen` crate; no private keys are committed as fixtures.
