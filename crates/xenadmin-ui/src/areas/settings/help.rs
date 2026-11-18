//! The Help topics: fixed text.
//!
//! The Rune topics described the Rune window (a row of buttons, a details
//! pane, an Activity dialog, a compact layout). These describe the shell the
//! client has now: menus, a toolbar, navigation views, a tab strip per
//! selected object and the Notifications view. Every fact of the Rune text is
//! kept; only the places changed. A paragraph names a control only when the
//! shell or the menus are known to have it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    Connect,
    Recovery,
    Console,
    Storage,
    Diagnostics,
}

impl Topic {
    pub const ALL: [Topic; 5] = [
        Topic::Connect,
        Topic::Recovery,
        Topic::Console,
        Topic::Storage,
        Topic::Diagnostics,
    ];

    /// The id of the topic in the topic chooser.
    pub fn id(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Recovery => "recovery",
            Self::Console => "console",
            Self::Storage => "storage",
            Self::Diagnostics => "diagnostics",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Connect => "Connections",
            Self::Recovery => "Operations and recovery",
            Self::Console => "Consoles",
            Self::Storage => "Snapshots and storage",
            Self::Diagnostics => "Development diagnostics",
        }
    }

    pub fn paragraphs(self) -> &'static [&'static str] {
        match self {
            Self::Connect => &[
                "Connect directly to an XCP-ng host management address. Any reachable member can discover its pool coordinator. Xen Orchestra is not required. The Home tab, shown when nothing is selected, lists the saved connections with their state. Add Server…, in the Server menu, on the toolbar and on the Home tab, saves a new one; Reconnect As…, in the Pool and Server menus, signs in to a pool as another user.",
                "Every connected pool is a root of the Infrastructure view of the navigation pane, with its servers, VMs, templates and storage repositories. The other views list the same objects by type, by organization and by saved search; the box above the tree narrows any of them by name or UUID. Selecting an object shows its tabs, and the menus, the toolbar and the context menu act on the selection.",
                "Use normal certificate verification, a trusted private CA or independently verified endpoint fingerprints. The validation opt-out applies to all destinations for that connection, including coordinator and console endpoints. Its status remains visible in the status bar.",
                "Passwords are session-only unless you explicitly save them in the desktop keyring. An unavailable or locked keyring leaves manual entry available. Automatic startup connections are configured per saved connection.",
            ],
            Self::Recovery => &[
                "Actions capture the selected object, pool and current connection. Review screens show the exact targets and consequences. The host checks capabilities again before execution; another operator or host task can change availability. A command that cannot run now stays in its menu, disabled, with the reason.",
                "A lost reply can leave Outcome unknown even when a host accepted the operation. Recheck outcome reads existing evidence; it does not repeat the original write. Request cancellation is a request to the host, not proof of cancellation.",
                "The Notifications view keeps task progress, completed steps and partial resources on its Events tab, and pool messages on its Alerts tab. The status bar counts running operations and unknown outcomes. Acknowledge an unknown outcome only after inspecting the host and stopping other clients handling that operation. Acknowledgement releases a local guard without proving the result.",
                "Force power actions always require review. Options, in the Tools menu, can also require a review for routine single-VM power actions. Bulk and configuration workflows retain their own target reviews.",
            ],
            Self::Console => &[
                "The Console tab sends input only to the VM whose console it shows; selecting another object never redirects keystrokes to it. The Console tab of a server is a text console on that server's verified control domain.",
                "The row of actions above the console offers fit or actual size, deliberate key chords, fullscreen, reconnect and Release input. Focus loss releases held keys and buttons. Clipboard transfer and screenshot saving happen only when requested. A console moved to its own window is listed in the Window menu.",
                "Automatic reconnect follows the same VM after a reboot or same-pool host change. It pauses for stale inventory or a stopped guest and stops after five failed attempts. Use Reconnect to restart attempts; cross-pool transfers offer an explicit destination reopen.",
                "Console defaults in Options apply to newly opened consoles. The current console keeps its own fit and reconnect choices.",
            ],
            Self::Storage => &[
                "A disk snapshot captures storage and configuration. A memory checkpoint also retains guest memory when the host supports it. Neither guarantees application consistency; guest-quiesced snapshots are unavailable on current XCP-ng targets.",
                "Restore reviews the exact parent and snapshot. Shutdown and start/resume choices are explicit. Snapshot deletion distinguishes its record from selected owned disk data; shared or unproven disks are retained.",
                "VM deletion, disk deletion, SR forgetting and SR destruction are separate operations. Read the selected UUIDs and retained-resource list. Growing a virtual disk does not grow partitions or filesystems inside the guest.",
                "Imports and cloud-init preparation retain created resource identities when interrupted. Cloud-init content can include secrets, so its local artifact stays private. Upload or attachment success does not prove the guest consumed that content.",
            ],
            Self::Diagnostics => &[
                "Check action availability reads the host's explanations for a VM's disabled power actions without executing them. Exports from the Events tab contain object names and identifiers; keep them private when investigating a real pool.",
                "Explain outcome, offered where an operation's outcome is shown and on the Events tab of the Notifications view, describes common host error codes and inspection steps. Its captured explanation keeps partial and unknown outcomes explicit; opening it does not reconnect, retry, change permissions or release an operation guard.",
                "Development builds can expose a same-user local inspection socket. Start with XENADMIN_DEV_INSPECT=1 and use the repository's scripts/inspect.sh helper. The inspector reads bounded client status and operation evidence; it does not submit host mutations or return passwords, sessions or console keystrokes.",
                "The read-only host-probe utility can inspect a connection or diagnose one VM. Prefer terminal password entry or a secret manager feeding standard input. Never put host passwords in command-line arguments or public reports.",
                "This development application separates fixture-tested behavior from live-host qualification. Destructive tests require a designated disposable lab. The repository's parity ledger and reports record current implementation and verification limits.",
            ],
        }
    }
}
