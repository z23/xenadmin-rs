# Connection fixes in 0.1.1

The first user connection attempt displayed `Invalid XAPI response: invalid inventory
number` before any inventory was published. No host capture or credential was supplied.

## Inventory failure

The core decoder assumed every size was unsigned. XAPI's
[SR introduction code](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_sr.ml)
initializes `physical_size` and `physical_utilisation` to `-1`. The reference client's SR
fields are also signed 64-bit integers. An introduced or unscanned SR can therefore prevent
the old decoder from publishing the entire pool inventory.

A fixture containing those legitimate values reproduced the exact reported error before
the change. The new decoder treats `-1`, missing and null SR sizes as unknown (`None`),
keeps known sizes exact, and displays **Unknown** instead of an invented zero. Known zero
usage displays **0 B**. Both JSON integers and decimal integer strings are supported.
Malformed numbers, fractional values, overflow and unsupported negative values still reject
the batch atomically. Errors now identify the affected class and field.

This is a reproduced compatibility defect and a likely explanation of the screenshot;
the actual user's host response has not been inspected or retested by the agent.

## Certificate opt-out

The connection form now has one **Disable certificate validation** checkbox. It applies
to the initial member, coordinator redirects, and authenticated console TLS endpoints.
The choice is remembered with the last endpoint. The connection status shows when it is
active. Pin and additional-host fields live under **Certificate options** when validation
is enabled. Existing preferences default to validation enabled.

The read-only probe supports `--insecure`. Both transports share the same verifier selection;
the opt-out overrides pins and custom CA configuration. HTTPS and TLS handshake signatures
remain enabled, while certificate chain, hostname, expiration and pin checks are skipped.
Console destination checks against authenticated pool host addresses remain in force.

## Local evidence

The check script passes formatting, Clippy with warnings denied, all **25 tests**, debug
workspace builds and four Rune screenshot/accessibility-tree exports. New checks cover:

- Bootstrap with integer and string `-1` storage sizes, followed by exact large and zero values.
- Malformed numeric diagnostics and atomic preservation of the prior inventory.
- Explicit opt-out with untrusted certificates and a mismatching configured pin.
- A real TLS member login, coordinator redirect and separate console CONNECT endpoint,
  each using a different self-signed certificate with a mismatching hostname. The fixture
  reaches authenticated CONNECT; existing RFB decoding/input tests remain separate.
- Real Rune checkbox interaction, restored access to verified certificate options, and
  unknown-versus-zero storage display.

No infrastructure operation or live-host request was made for this fix. Host compatibility
still needs the user's connection retry. Remote CI and packaged-artifact results should be
recorded separately when observed.
