# Connection trust and proxy routing — 2026-11-08

Connections now offer certificate inspection and reviewed acceptance or replacement of an
exact endpoint's SHA-256 pin. Saved options include Direct, Environment and HTTP CONNECT
proxy routes. Connection failures show their login, inventory or event-stream stage, the
effective route and the exact endpoint when certificate validation failed.

## Certificate review

Inspection performs a bounded TLS handshake without an XAPI request, login password or
session cookie. It displays the leaf SHA-256, subject, issuer, alternative names, validity
period and serial, together with the normal platform/custom-CA verification result. The
inspection-only verifier permits examining an untrusted certificate while still checking
handshake signatures. Normal connections keep enforcing their configured trust policy.

The user must confirm the exact normalized endpoint and that the SHA-256 was verified
through a trusted channel. A changed certificate review includes the previous fingerprint
and any stored certificate summary. Acceptance uses a compare-and-replace check against
the previous pin and updates only the unsaved editor draft. It does not save, connect,
retry or trust a redirected/member endpoint implicitly. A failed-member review does not
load the saved host password. Changed form, endpoint, CA, pin, proxy, proxy password or
editor epoch invalidates delayed inspection results and review controls.

An accepted pin trusts that exact certificate even if its CA, hostname or validity checks
fail; the UI makes this explicit. Inspection alone does not establish endpoint ownership.
Certificate replacement between inspection and login is rejected before host credentials
are sent. Version 3 profiles store bounded certificate summaries and proxy configuration;
versions 1 and 2 remain readable. Profiles and their size limits retain private atomic
persistence, and no proxy passwords are serialized.

## Proxy boundary

Environment routing captures HTTP_PROXY, HTTPS_PROXY, ALL_PROXY and NO_PROXY once per
connection, including their supported lowercase forms. HTTP(S)_PROXY selects the destination
scheme; the configured proxy itself must use HTTP. Direct routing disables environment
discovery. Explicit proxies accept an HTTP host and port, with an optional username and
session-only Basic password. Credentials bind to the exact proxy endpoint and username in
both the UI cache and core API. Proxy route and credential authority participate in the
authenticated pool-owner key, preventing aliases with different authority from sharing an
owner silently.

All HTTP clients and the raw console/inspection socket path use the same captured routing
policy. The latter implements bounded HTTP CONNECT with optional proxy authorization,
then performs normal host TLS. Proxy credentials stay in the outer CONNECT request; host
login/session data stays inside TLS. HTTP proxy authentication itself is sent without TLS
to the proxy, which the editor states. HTTPS proxy, SOCKS and PAC configurations are
explicitly unsupported. Refusal, authentication failure, redirect or malformed/oversized
proxy responses never cause a direct fallback.

This increment does not add proxy password persistence, Digest/NTLM/Kerberos negotiation,
OS PAC discovery, certificate-chain management or automatic trust repair. Existing request
timeouts remain configurable. Error stage and route are diagnostics, not proof of a root
cause; live proxy, distribution trust-store and multihost qualification remain open.

## Verification

- 676 workspace tests passed: 324 core unit/protocol, 174 direct connection, 165 desktop,
  five profile, four TLS and four host-probe tests.
- Five new loopback TLS/proxy fixtures cover credential-free inspection and custom CA,
  certificate rotation before login, shared proxy routing and credential separation,
  stale pin replacement and persistence, and refused/redirected/oversized CONNECT responses.
- Two proxy unit tests cover frozen environment/NO_PROXY behavior, unsupported schemes,
  redacted diagnostics, changed authority and same-username/different-endpoint credentials.
- Six Rune tests cover compact/wide controls, exact endpoint/fingerprint confirmation,
  delayed-result invalidation, transient proxy credentials, synthetic no-network behavior
  and failed-member review without loading a host password.
- Strict workspace Clippy, formatting, whitespace and shell syntax checks passed. The
  workspace desktop build passed.
- Four synthetic Vulkan renders at 1440×900 and 513×652 were visually inspected. Dialog
  headings and footer actions remain visible while the form/review body scrolls. Interaction
  tests reach the confirmation controls by scrolling. Scenes are in `scripts/check.sh`.

These tests use generated certificates, loopback fixtures and example endpoint names.
No live host was contacted and no private screenshots, inventories or credentials were
published. Headless renders do not establish native Wayland/X11, keyboard or Orca behavior.

## Reference evidence

XenAdmin's [unknown-certificate dialog](https://github.com/xcp-ng/xenadmin/blob/master/XenAdmin/Dialogs/Network/UnknownCertificateDialog.cs)
and [changed-certificate dialog](https://github.com/xcp-ng/xenadmin/blob/master/XenAdmin/Dialogs/Network/CertificateChangedDialog.cs)
provide the behavioral reference for explicit inspection and replacement. This implementation
uses exact normalized endpoints and SHA-256 pins.

[Reqwest proxy configuration](https://docs.rs/reqwest/0.13.5/reqwest/struct.Proxy.html)
and [client configuration](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html)
document explicit proxy and automatic-discovery control. Captured environment matching uses
[hyper-util's matcher](https://docs.rs/hyper-util/0.1.21/hyper_util/client/proxy/matcher/struct.Matcher.html).
The implementation separates [rustls certificate and handshake-signature checks](https://docs.rs/rustls/0.23.37/rustls/client/danger/trait.ServerCertVerifier.html)
and uses the same [platform verifier](https://docs.rs/rustls-platform-verifier/0.7.1/rustls_platform_verifier/struct.Verifier.html)
for inspection results and normal CA verification. Bounded leaf metadata parsing uses
[`x509-parser` 0.18.1](https://docs.rs/x509-parser/0.18.1/x509_parser/); `Cargo.lock` records
the exact dependency graph.
