# Saved connections and desktop passwords

XenAdmin RS connects directly to XCP-ng management endpoints over HTTPS. Xen Orchestra is not involved.

## Save and connect

Use **Add connection** for a new host, or **Connections → Connect / edit** for a saved host. Enter a management hostname or IP address, username and password. A connection name is optional. **Saved connection options** contains a custom CA certificate path, a request timeout from 5 to 300 seconds, read-only mode and **Save without connecting**.

The first launch migrates the last host settings from `preferences.json` into `profiles.json`. Passwords are not imported. Profiles preserve their IDs when edited. A profile can be edited or removed after its live connection is disconnected.

**Connections** shows saved entries and their connection status. Several different pools can remain connected. The sidebar buttons switch the displayed pool; switching clears the selected object, property dialogs and current console so an action cannot accidentally retain a target from the previous pool. Inventory and operation tracking continue in the background.

Connecting to another endpoint of an already connected pool authenticates the endpoint before sharing the existing pool connection. Sharing requires the same credentials, trust configuration and read-only setting. An incompatible connection fails explicitly; disconnect the pool before changing its authentication context. Disconnecting either same-pool entry disconnects the shared pool session.

Once an endpoint has successfully returned the expected pool, the profile records that pool identity and the endpoints used to reach it. Reconnect can try these remembered members if the primary endpoint is unavailable. This is not discovery of every advertised host address: only previously authenticated endpoints are fallback candidates. An endpoint returning a different pool fails instead of replacing the displayed inventory.

## Certificates

CA and hostname validation are enabled by default. For a private CA, enter its PEM file under **Saved connection options**. **Certificate options** supports a verified SHA-256 fingerprint for the initial host and separate fingerprints for other pool members.

**Disable certificate validation** applies to the connection, coordinator redirects and consoles. HTTPS remains in use, but the server identity is not verified. This setting is saved per profile and changing the host address clears it. The connected view displays when verification is disabled.

## Password storage

The default is session-only credentials. Passwords are never written to the profile, preferences or operation database.

Select **Remember password in desktop keyring** to use the desktop's Secret Service password manager. Connecting saves a newly entered password only after the host has authenticated and supplied a usable inventory. **Save without connecting** explicitly saves the entered password without checking it against the host.

Opening a saved profile with keyring storage enabled attempts to load its password. The password field stays empty; the form says when a saved password has loaded. Leave it empty to use the loaded password, or enter a replacement. Stored credentials are bound to the profile ID, endpoint and username. Editing the address or username cannot silently reuse another binding.

**Load saved password** retries a lookup. **Forget saved password** removes all keyring entries belonging to the profile, including previous endpoint/account bindings, and returns the profile to session-only mode. Removing a profile with keyring storage enabled first removes its saved passwords; a locked or unavailable keyring leaves the profile available for retry.

If the password manager is unavailable, locked or denies access, the form reports that condition and accepts a password for the current session. Unlock it through the desktop password manager and retry if desired. XenAdmin RS does not silently write an unencrypted fallback file. Keyring calls run away from the UI thread and have a bounded timeout. They use the Secret Service `plain` transfer algorithm on the local session D-Bus connection; the password manager controls at-rest encryption and unlocking.

## State ownership and privacy

The state directory is `XENADMIN_STATE_DIR`, or `$XDG_STATE_HOME/xenadmin-rs`, or `~/.local/state/xenadmin-rs`. It contains connection metadata and local operation history, which can include sensitive host and VM names. It is not an export or diagnostic attachment.

One desktop window owns an advisory lock for a state directory. Another window reports that the directory is already in use and cannot connect or change saved settings. Close the first window before using that directory in another instance.

Profiles use a versioned document and atomic replacement. Unknown newer profile formats fail without being overwritten. The state directory and newly written profile, preference and lock files use restrictive owner permissions.

## Validation and remaining limitations

Tests use synthetic hosts, a private in-process Secret Service mock and Rune widget interaction. They cover profile migration, secret exclusion, pool identity binding, same-pool ownership, reconnect fallback, clone lifetimes, profile editing, instance locking and keyring lookup/save/removal failures. They do not exercise a user's actual password manager or production pool.

The client currently asks the user to unlock a locked keyring in the desktop password manager; it does not display Secret Service unlock prompts itself. There is no bulk profile import/export or automatic connection on startup.
