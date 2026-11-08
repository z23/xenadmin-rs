# Direct host text consoles

Host details offer **Open host text console** when current inventory identifies the host's
running control domain. The connection facade captures both host observation and expected
control-domain UUID. Immediately before discovery it resolves the host UUID again, obtains
the current control domain, and verifies its UUID, resident host, running state and explicit
control-domain/non-template/non-snapshot flags. The selected RFB console must belong to that
VM. Missing or changed identity stops opening; it never falls back to another guest.

The route follows the [XAPI host control-domain API](https://xapi-project.github.io/xen-api/classes/host.html#get_control_domain)
and [console ownership/protocol API](https://xapi-project.github.io/xen-api/classes/console.html).
The reference XenAdmin uses the control-domain VM for its host console. This implementation
reuses the direct authenticated tunnel and existing endpoint trust policy, with no XO or
second login. Discovery is bounded to 32 console records and validated opaque references.
Closing during discovery, TLS connection or RFB negotiation cancels the opening future.

The console pins host and control-domain identities. Details remain on the host even when
table selection or inventory tabs change. VM power, property, device and snapshot controls
are absent. The host toolbar offers Ctrl+C/Ctrl+D rather than guest reboot/key-switch
shortcuts. Clipboard, image export, input release and bounded lifecycle recovery remain
available. A replaced control-domain UUID requires opening the host console again explicitly.

## Verification

Five console protocol tests passed, including the existing guest and new host authenticated
CONNECT/RFB handshake and input roundtrips; ten refusal cases cover wrong UUID, resident host,
kind, power state, snapshot flag, console ownership, malformed references and oversized lists.
Additional checks cover read-only/stale/wrong-control-domain admission and closing while
discovery is pending. The TLS redirect fixture includes the real console.VM relationship.

The source worktree passed all 627 workspace tests (4 probe, 302 core unit, 5 profiles,
163 direct protocol, 4 TLS and 149 desktop), strict Clippy and compilation. A Rune fixture
checks host-only details and pinned selection at wide/compact sizes. Synthetic native renders
were inspected. No production console was opened or host command executed. Actual XCP-ng
host text rendering and input await the prepared disposable lab.
