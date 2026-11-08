# Secondary interface configuration

This increment adds reviewed IPv6 settings, interface purpose labels and unplug protection
for non-management PIFs. It extends the existing network command journal; it does not yet
provide management handover, pool VLAN retagging or management-bond migration.

## Behavior

The IPv6 editor supports None, DHCP, Autoconf and one static address/prefix with optional
gateway. DNS is shared with the interface's IPv4 configuration and the review says so.
The backend requires a freshly observed IPv6-enabled management PIF on the same host:
it never attempts to enable IPv6 on an installation configured without it. It rejects
unmanaged PIFs, bond members, changed identities and changed pre-edit settings.
Address-set comparison accepts equivalent IPv6 notation; the desired static address must
be present on verification. The existing IPv4 configuration is left intact.

Purpose is the XenAdmin-compatible `PIF.other_config[management_purpose]` label. A fresh map
merge preserves other keys. Changing the purpose does not silently enable unplug protection
or change the IP configuration. Protection is a separate reviewed `PIF.set_disallow_unplug`
command. Protected interfaces cannot be unplugged through this client until the protection
is explicitly removed. The server remains authoritative for all operations.

All three writes use asynchronous XAPI tasks and the existing pool reservation. A lost
submission receipt remains unresolved; rechecking never repeats it. Public PIF inventory
now includes managed status, unplug protection, purpose and primary address type.

## Compatibility decisions

XCP-ng documents nonstandard management MTUs as unsupported, so the interface view makes
that limitation explicit and the existing backend refusal remains. The IPv6 address family
selected during installation cannot be changed through this editor. These are supported
platform boundaries, not hidden completed features. See [XCP-ng networking](https://docs.xcp-ng.org/networking/),
[XCP-ng installation](https://docs.xcp-ng.org/installation/install-xcp-ng/), and
[XCP-ng 8.3 release notes](https://docs.xcp-ng.org/releases/release-8-3/).

The API contracts are [PIF.reconfigure_ipv6 and PIF setters](https://xapi-project.github.io/xen-api/classes/pif.html).
Local XenAdmin references reviewed: `XenModel/Actions/Network/NetworkingActionHelpers.cs`
and `XenModel/XenAPI-Extensions/PIF.cs`. The implementation is original Rust and Rune code.

## Verification

Synthetic protocol coverage checks preserved IPv4 fields and unrelated map keys, purpose
removal, protection, disabled-host/changed-address/bond-member refusal, concurrent purpose
changes, and lost-reply reconciliation without replay. Rune tests cover captured identity,
pool/configuration changes, invalid prefixes and review/confirmation controls at 513×652
and 1440×900. Demo previews cannot dispatch. No live hosts, private screenshots or credentials
were used. XCP-ng lab qualification remains deferred by the user.

The complete workspace passed 653 tests: 305 core unit, 177 protocol, five profile,
four TLS, four CLI and 158 desktop tests. This includes four new protocol cases and
three new Rune cases. Strict workspace all-target Clippy, formatting and diff checks passed.
Native Vulkan renders of the synthetic IPv6 form were inspected at both supported review
sizes; `scripts/check.sh` now includes the `network-ipv6` scene.
