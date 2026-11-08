# Guest SSH/RDP and trusted local tools

Running ordinary guests now have **Guest SSH, RDP and tools…** in Details. Choose a tool,
select a guest-reported address or explicitly enter a manually verified literal IP, and enter
the guest username and port. **Review command** shows the resolved local executable and every
argument separately. **Launch reviewed tool** starts it once. The guest username is separate
from the XAPI host login; this application has no guest-password field.

The initial presets use `/usr/bin/xdg-terminal-exec` with OpenSSH, and `/usr/bin/remmina` with
an RDP URI. Missing clients produce an error rather than selecting another program. Configure
absolute executable paths and JSON argument arrays in the launch dialog or Options. Add,
edit and remove up to sixteen declarations. Save persists them in private atomic preferences;
nothing is discovered, downloaded, installed or executed automatically. Literal argument
strings can contain spaces; the application never shell-splits or evaluates them.

Version 1 supports `{ip}`, `{port}`, `{user}`, `{vm_uuid}`, `{pool_uuid}` and `{rdp_uri}`.
Each expansion stays inside its original argument. The URI encoder escapes usernames and
brackets IPv6 addresses. Unknown placeholders, extra credential fields, endpoint/session placeholders,
control characters, relative executable paths and overlarge declarations are refused.
Usernames use a conservative letters/digits/`._-@\` subset; hostnames, IPv6 scope IDs and
link-local IPv6 destinations are not supported. A tool can ignore user/port placeholders
when its own workflow does not require them.

Local programs are explicitly trusted and run as the Linux user. This is not a sandbox.
Only desktop, locale, user-home and SSH-agent environment variables are inherited, with a
fixed system executable search path; no XAPI credentials, session tokens or raw records are
provided. stdin/stdout/stderr are disconnected from the application. Guest authentication,
SSH host-key checks, RDP certificate policy and any client-specific saved credentials belong
to the selected client. Guest-reported IP addresses do not prove server identity.

The launch target captures its connection, pool, VM and generation. Review preparation is
bounded background work, so inspecting an unavailable filesystem path does not block Rune.
Closing or changing the review discards late results. Immediately before subprocess creation,
the worker verifies the reviewed executable's file identity and the current captured pool,
generation, running ordinary guest and, when selected, current guest-reported address. Main
selection changes never silently retarget a prepared command. Manual addresses remain an
explicit operator assertion. These checks do not prove that a remote server is the guest.

At most eight review/process workers run simultaneously. Local launch/exit notices remain in
memory and do not claim a successful guest login. An external launcher may exit while its
client window remains open. Closing the dialog or application does not terminate that client.
There is no retry after an ambiguous local launch or automatic remote command.

## Evidence and boundaries

The full workspace passed 971 tests (432 core units, 262 direct protocol, 264 desktop,
5 profiles, 4 TLS and 4 probe). Eight focused fixtures cover argv preservation with literal
shell metacharacters, allowlisted environment, executable replacement, mapped IPv6 filtering,
RDP URI encoding, unknown/secret placeholders, version/bounds checks, target freshness,
delayed review discard and compact/wide Rune flows. A synthetic local executable writes its
received argv/environment for assertions; no guest client or network connection is launched.
Strict workspace/all-target Clippy, release desktop compilation, formatting and diff checks
passed. Synthetic compact launch/configuration and wide command-review GPU renders were
inspected. Scenes force synthetic tool declarations rather than reading user-configured tools.

Source review by another agent found no blocker; its mapped-IP and asynchronous-filesystem
suggestions were implemented. Live SSH/RDP login, terminal/Remmina availability across
supported distributions, client certificate prompts and real guest-address behavior remain
lab qualification tasks. The bounded trusted subprocess declaration is the implemented native
extension boundary. Automatic plugin discovery, declarative extension views, richer capability
APIs and Windows plugin compatibility are not implemented; Windows compatibility remains
excluded from scope.

## Primary references

- [OpenSSH client manual](https://man.openbsd.org/ssh.1): login-name/port arguments and client authentication.
- [xdg-terminal-exec reference implementation](https://github.com/Vladimir-csp/xdg-terminal-exec): separate command/argument execution through the preferred terminal.
- [Remmina command-line source](https://remmina.gitlab.io/remminadoc.gitlab.io/remmina_8c_source.html): quick-connect RDP URI support.
- The local XenAdmin `Plugins/Cmd/ShellCmd.cs` and `Plugins/Placeholders.cs` informed the behavioral audit; no Windows plugin or shell-command implementation was copied.
