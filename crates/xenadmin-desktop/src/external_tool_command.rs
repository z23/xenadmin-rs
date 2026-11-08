use serde::{Deserialize, Serialize};
use std::{net::IpAddr, path::PathBuf};
use xenadmin_core::{PoolId, VmId};

pub const MAX_TOOLS: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Ssh,
    Rdp,
    Custom,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub id: uuid::Uuid,
    pub name: String,
    pub kind: Kind,
    pub executable: String,
    pub arguments: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u8,
    pub tools: Vec<Tool>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            tools: vec![Tool::preset(Kind::Ssh), Tool::preset(Kind::Rdp)],
        }
    }
}
impl Catalog {
    pub fn valid(&self) -> bool {
        self.version == 1
            && self.tools.len() <= MAX_TOOLS
            && self.tools.iter().all(|t| t.validate().is_ok())
            && self
                .tools
                .iter()
                .map(|t| t.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.tools.len()
    }
}
impl Tool {
    pub fn preset(kind: Kind) -> Self {
        let (name, executable, arguments) = match kind {
            Kind::Ssh => (
                "SSH in default terminal",
                "/usr/bin/xdg-terminal-exec",
                vec![
                    "--",
                    "/usr/bin/ssh",
                    "-p",
                    "{port}",
                    "-l",
                    "{user}",
                    "--",
                    "{ip}",
                ],
            ),
            Kind::Rdp => (
                "RDP with Remmina",
                "/usr/bin/remmina",
                vec!["--connect", "{rdp_uri}"],
            ),
            Kind::Custom => ("Local guest tool", "/usr/bin/your-tool", vec!["{ip}"]),
        };
        Self {
            id: uuid::Uuid::new_v4(),
            name: name.into(),
            kind,
            executable: executable.into(),
            arguments: arguments.into_iter().map(str::to_owned).collect(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || self.name.trim().is_empty()
            || !text(&self.name, 80)
            || !text(&self.executable, 4096)
            || !std::path::Path::new(&self.executable).is_absolute()
            || self.arguments.len() > 32
            || self.arguments.iter().any(|s| !text(s, 2048))
            || self.arguments.iter().map(String::len).sum::<usize>() > 16_384
        {
            return Err("Use a name, an absolute executable path and at most 32 arguments (16 KiB total). Control characters are not allowed.".into());
        }
        for arg in &self.arguments {
            expand(arg, &Context::fixture())?;
        }
        Ok(())
    }
}
fn text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.chars().any(char::is_control)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub ip: IpAddr,
    pub port: u16,
    pub user: String,
    pub vm: VmId,
    pub pool: PoolId,
}
impl Context {
    fn fixture() -> Self {
        Self {
            ip: "192.0.2.1".parse().unwrap(),
            port: 22,
            user: "operator".into(),
            vm: VmId::from(uuid::Uuid::from_u128(1)),
            pool: PoolId::from(uuid::Uuid::from_u128(2)),
        }
    }
    fn valid(&self) -> bool {
        usable_ip(self.ip)
            && self.port > 0
            && self.user.len() <= 128
            && !self.user.starts_with('-')
            && self
                .user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@\\".contains(&b))
    }
    fn rdp_uri(&self) -> String {
        let user = self
            .user
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>();
        let user = if user.is_empty() {
            user
        } else {
            format!("{user}@")
        };
        let address = match self.ip {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        format!("rdp://{user}{address}:{}", self.port)
    }
}
pub fn usable_ip(ip: IpAddr) -> bool {
    if let IpAddr::V6(v) = ip
        && let Some(mapped) = v.to_ipv4_mapped()
    {
        return usable_ip(IpAddr::V4(mapped));
    }
    !ip.is_unspecified()
        && !ip.is_multicast()
        && !ip.is_loopback()
        && match ip {
            IpAddr::V4(v) => !v.is_broadcast(),
            IpAddr::V6(v) => !v.is_unicast_link_local(),
        }
}
fn expand(template: &str, context: &Context) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        if rest[..start].contains('}') {
            return Err("Unmatched placeholder brace in arguments".into());
        }
        out.push_str(&rest[..start]);
        let close = rest[start..]
            .find('}')
            .ok_or("Unclosed placeholder in arguments")?
            + start;
        let value=match &rest[start+1..close] {
            "ip"=>context.ip.to_string(),"port"=>context.port.to_string(),"user"=>context.user.clone(),
            "vm_uuid"=>context.vm.to_string(),"pool_uuid"=>context.pool.to_string(),"rdp_uri"=>context.rdp_uri(),
            _=>return Err("Supported placeholders: {ip}, {port}, {user}, {vm_uuid}, {pool_uuid}, {rdp_uri}. Credentials and host sessions are not available.".into()),
        };
        out.push_str(&value);
        rest = &rest[close + 1..];
    }
    if rest.contains('}') {
        return Err("Unmatched placeholder brace in arguments".into());
    }
    out.push_str(rest);
    Ok(out)
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    len: u64,
    modified: i64,
    nanos: i64,
    mode: u32,
}
fn executable(path: &str) -> Result<(PathBuf, Identity), String> {
    use std::os::unix::fs::MetadataExt;
    let path = std::fs::canonicalize(path)
        .map_err(|_| "Executable not found. Install the client or configure its absolute path.")?;
    let m = path
        .metadata()
        .map_err(|_| "Cannot inspect the configured executable")?;
    if !m.is_file() || m.mode() & 0o111 == 0 {
        return Err("The configured path is not an executable file".into());
    }
    Ok((
        path,
        Identity {
            device: m.dev(),
            inode: m.ino(),
            len: m.len(),
            modified: m.mtime(),
            nanos: m.mtime_nsec(),
            mode: m.mode(),
        },
    ))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    identity: Identity,
}
impl Command {
    pub fn prepare(tool: &Tool, context: &Context) -> Result<Self, String> {
        tool.validate()?;
        if !context.valid() {
            return Err("Choose a usable guest IP, a port from 1–65535 and a username containing letters, digits, . _ - @ or \\.".into());
        }
        if tool.arguments.iter().any(|a| a.contains("{user}")) && context.user.is_empty() {
            return Err("Enter the guest username required by this tool".into());
        }
        let arguments = tool
            .arguments
            .iter()
            .map(|a| expand(a, context))
            .collect::<Result<Vec<_>, _>>()?;
        if arguments.iter().map(String::len).sum::<usize>() > 16_384 {
            return Err("Expanded arguments exceed 16 KiB".into());
        }
        let (executable, identity) = executable(&tool.executable)?;
        Ok(Self {
            executable,
            arguments,
            identity,
        })
    }
    pub fn spawn(
        &self,
        check: impl FnOnce() -> Result<(), String>,
    ) -> Result<std::process::Child, String> {
        use std::os::unix::process::CommandExt;
        let (path, identity) = executable(
            self.executable
                .to_str()
                .ok_or("Executable path is not UTF-8")?,
        )?;
        if path != self.executable || identity != self.identity {
            return Err("The executable changed after review. Review the command again.".into());
        }
        let mut command = std::process::Command::new(&path);
        command
            .args(&self.arguments)
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .current_dir("/")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        for key in [
            "HOME",
            "USER",
            "LOGNAME",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_TYPE",
            "XAUTHORITY",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "SSH_AUTH_SOCK",
            "XDG_CONFIG_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_DATA_HOME",
            "XDG_DATA_DIRS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        check()?;
        command
            .spawn()
            .map_err(|e| format!("Cannot launch executable: {}", e.kind()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn fixture(dir: &std::path::Path) -> Tool {
        let path = dir.join("tool.py");
        std::fs::write(&path,"#!/usr/bin/python3\nimport sys,os,json\nwith open(sys.argv[1],'w') as f: json.dump({'args':sys.argv[2:],'env':dict(os.environ)},f)\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Tool {
            id: uuid::Uuid::new_v4(),
            name: "Fixture only".into(),
            kind: Kind::Custom,
            executable: path.to_str().unwrap().into(),
            arguments: vec![
                dir.join("result.json").to_str().unwrap().into(),
                "literal space; $(touch should-not-exist) `echo nope` *".into(),
                "{ip}".into(),
                "{vm_uuid}".into(),
            ],
        }
    }
    #[test]
    fn local_launch_preserves_argv_without_shell_or_secret_environment() {
        let dir = tempfile::tempdir().unwrap();
        let tool = fixture(dir.path());
        let context = Context::fixture();
        let command = Command::prepare(&tool, &context).unwrap();
        assert!(
            command
                .spawn(|| Err("Changed guest before exec".into()))
                .is_err()
        );
        assert!(!dir.path().join("result.json").exists());
        assert!(command.spawn(|| Ok(())).unwrap().wait().unwrap().success());
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("result.json")).unwrap())
                .unwrap();
        assert_eq!(result["args"][0], tool.arguments[1]);
        assert_eq!(result["args"][1], "192.0.2.1");
        assert_eq!(result["args"][2], context.vm.to_string());
        let env = result["env"].as_object().unwrap();
        assert!(env.keys().all(|k| {
            [
                "PATH",
                "HOME",
                "USER",
                "LOGNAME",
                "DISPLAY",
                "WAYLAND_DISPLAY",
                "XDG_RUNTIME_DIR",
                "DBUS_SESSION_BUS_ADDRESS",
                "XDG_CURRENT_DESKTOP",
                "XDG_SESSION_TYPE",
                "XAUTHORITY",
                "LANG",
                "LC_ALL",
                "LC_CTYPE",
                "SSH_AUTH_SOCK",
                "XDG_CONFIG_HOME",
                "XDG_CONFIG_DIRS",
                "XDG_DATA_HOME",
                "XDG_DATA_DIRS",
            ]
            .contains(&k.as_str())
        }));
        assert!(
            !env.contains_key("XENADMIN_STATE_DIR") && !env.contains_key("XENADMIN_DEV_INSPECT")
        );
    }
    #[test]
    fn command_requires_reviewed_executable_and_bounded_typed_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let mut tool = fixture(dir.path());
        let mut context = Context::fixture();
        let command = Command::prepare(&tool, &context).unwrap();
        std::fs::write(&tool.executable, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(command.spawn(|| Ok(())).is_err());
        for user in [
            "-oProxyCommand=bad",
            "operator;touch bad",
            "user\nname",
            "user$(bad)",
        ] {
            context.user = user.into();
            assert!(Command::prepare(&tool, &context).is_err());
        }
        context = Context::fixture();
        for address in [
            "127.0.0.1",
            "::ffff:127.0.0.1",
            "::ffff:224.0.0.1",
            "::",
            "::ffff:0.0.0.0",
            "fe80::1",
        ] {
            context.ip = address.parse().unwrap();
            assert!(Command::prepare(&tool, &context).is_err());
        }
        context = Context::fixture();
        tool.arguments = vec!["{session_id}".into()];
        assert!(tool.validate().is_err());
        tool.arguments = vec!["{ip}".repeat(3000)];
        assert!(tool.validate().is_err());
        tool.arguments = vec!["{ip}".into(); 33];
        assert!(tool.validate().is_err());
        tool.arguments = vec![];
        tool.executable = "relative-tool".into();
        assert!(Command::prepare(&tool, &context).is_err());
    }
    #[test]
    fn rdp_uri_escapes_username_and_brackets_ipv6_without_password() {
        let mut c = Context::fixture();
        c.user = "EXAMPLE\\operator@example.test".into();
        c.ip = "2001:db8::3".parse().unwrap();
        c.port = 3389;
        assert_eq!(
            expand("{rdp_uri}", &c).unwrap(),
            "rdp://EXAMPLE%5Coperator%40example.test@[2001:db8::3]:3389"
        );
        c.user.clear();
        assert_eq!(c.rdp_uri(), "rdp://[2001:db8::3]:3389");
        let ssh = Tool::preset(Kind::Ssh);
        assert_eq!(ssh.arguments.last().unwrap(), "{ip}");
        assert_eq!(ssh.arguments[ssh.arguments.len() - 2], "--");
        assert!(Command::prepare(&ssh, &c).is_err());
    }
    #[test]
    fn local_declarations_are_versioned_bounded_and_do_not_accept_hidden_fields() {
        let mut catalog = Catalog::default();
        assert!(catalog.valid());
        let json = serde_json::to_string(&catalog).unwrap();
        assert!(serde_json::from_str::<Catalog>(&json).unwrap().valid());
        catalog.tools.push(catalog.tools[0].clone());
        assert!(!catalog.valid());
        catalog = Catalog::default();
        catalog.version = 2;
        assert!(!catalog.valid());
        let mut tool = serde_json::to_value(Tool::preset(Kind::Custom)).unwrap();
        tool["password"] = serde_json::json!("never accepted");
        assert!(serde_json::from_value::<Tool>(tool).is_err());
        for token in [
            "{password}",
            "{session}",
            "{endpoint}",
            "{vm_name}",
            "{xapi}",
            "{ip",
            "ip}",
        ] {
            assert!(expand(token, &Context::fixture()).is_err());
        }
    }
}
