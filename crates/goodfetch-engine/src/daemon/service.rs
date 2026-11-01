//! The per-user service `daemon install` registers for a major (ADRs 0024,
//! 0031, 0059 and 0098), as pure functions of what it is given, which
//! test support snapshots: a launch agent with socket activation on macOS,
//! a systemd user socket and service on Linux, and a logon task on Windows.
//! An executable writes the files a [`ServicePlan`] lists and runs its
//! commands; nothing here touches the machine.
//!
//! The service starts the versioned copy of the executable that installed
//! it, in the Browser cache, with `daemon launched --service`. That copy
//! starts its major's pick: itself when it is the pick, and otherwise the
//! pick's own versioned copy, which on macOS and Linux takes its place in
//! the same process, so the socket the service manager hands over and the
//! process it watches stay the Daemon's. The list of installed executables
//! records the copy as an executable at its own path, so the clean-up keeps
//! it while it is there, until `daemon uninstall` forgets it.
//!
//! The service's name holds the major and the first digits of the digest
//! of Goodfetch home's path, so each major of each Goodfetch home has its
//! own, and tests, each with its own home, never meet. The deployment
//! variables that place Goodfetch home, the Browser cache and the logs go
//! to the Daemon as arguments of `daemon launched`, since a logon task
//! sets no environment; no secret is among them.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{InstalledService, ServiceManager};
use sha2::{Digest as _, Sha256};

use crate::home::{Home, major_name};
use crate::transport::{HeldPort, LAUNCHD_CONSOLE_SOCKET, LAUNCHD_SOCKETS, hold_free_port};

/// The deployment variables a service passes on to the Daemon it starts:
/// where Goodfetch home and the Browser cache are. Where its logs go is
/// the `log.*` settings' (ADR 0122), which it reads from its home.
pub const PASSED_VARIABLES: [&str; 2] = ["GOODFETCH_BROWSERS_PATH", "GOODFETCH_HOME"];

/// How many hex digits of Goodfetch home's digest a service's name holds.
const HOME_DIGITS: usize = 8;

/// The service manager of the OS this was built for, or `None` where
/// Goodfetch registers no service.
pub fn manager_of_this_os() -> Option<ServiceManager> {
    if cfg!(target_os = "macos") {
        Some(ServiceManager::Launchd)
    } else if cfg!(target_os = "linux") {
        Some(ServiceManager::Systemd)
    } else if cfg!(windows) {
        Some(ServiceManager::TaskScheduler)
    } else {
        None
    }
}

/// Whether the service manager hands the Daemon its socket at the first
/// connection: launchd and systemd do, a logon task starts it at logon.
pub fn activates_sockets(manager: ServiceManager) -> bool {
    matches!(manager, ServiceManager::Launchd | ServiceManager::Systemd)
}

/// The name of the service of `version`'s major for Goodfetch home `home`
/// under `manager`: a reverse-DNS launch agent label, a user unit's name,
/// or a logon task's path in the `Goodfetch` folder.
pub fn service_name(manager: ServiceManager, home: &Home, version: &ProductVersion) -> String {
    let major = major_name(version);
    let digest = Sha256::digest(home.root().as_os_str().as_encoded_bytes());
    let home = digest.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    let home = &home[..HOME_DIGITS];
    match manager {
        ServiceManager::Launchd => format!("dev.goodfetch.daemon.{major}.{home}"),
        ServiceManager::Systemd => format!("goodfetch-daemon-{major}-{home}"),
        ServiceManager::TaskScheduler => format!("Goodfetch\\goodfetch-daemon-{major}-{home}"),
    }
}

/// The Console's address a service manager listens on for the Daemon it
/// starts (ADR 0031), which must outlive the registration: a port chosen
/// for port 0 stays held until the manager listens on it, so keep this until
/// [`ServicePlan::register`]'s commands have returned.
#[derive(Debug)]
pub struct ConsoleSocket {
    address: SocketAddr,
    hold: Option<HeldPort>,
}

impl ConsoleSocket {
    /// What `manager` listens on for `console.address`. A named port is taken
    /// as it is, and nothing shares it: a second listener is refused as with
    /// any port. Port 0 takes a port held for systemd, whose socket unit joins
    /// the hold (#1821). launchd has no such key and neither chooses a port it
    /// can report, so port 0 gets none from it, the Console's address left to
    /// the Daemon; nor does a manager that activates no socket.
    ///
    /// # Errors
    ///
    /// When no port can be held on the address's host.
    pub fn of(manager: ServiceManager, address: SocketAddr) -> std::io::Result<Option<Self>> {
        if !activates_sockets(manager) {
            return Ok(None);
        }
        if address.port() != 0 {
            return Ok(Some(Self {
                address,
                hold: None,
            }));
        }
        if manager != ServiceManager::Systemd {
            return Ok(None);
        }
        let hold = hold_free_port(address.ip())?;
        Ok(Some(Self {
            address: hold.address(),
            hold: Some(hold),
        }))
    }

    /// The address, with the port held in place of port 0.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

/// What a service is registered from.
#[derive(Clone, Copy, Debug)]
pub struct ServiceRequest<'a> {
    /// The service manager to register it with.
    pub manager: ServiceManager,
    /// Goodfetch home.
    pub home: &'a Home,
    /// The Product version of the executable that installs it, whose major
    /// it serves.
    pub version: &'a ProductVersion,
    /// The versioned copy it starts.
    pub executable: &'a Path,
    /// That copy's build in the Browser cache.
    pub build: &'a str,
    /// The socket the service manager listens on, with socket activation.
    pub socket: Option<&'a Path>,
    /// The Console's address the service manager also listens on, with
    /// socket activation, so the first visit to the Console starts the
    /// Daemon too (ADR 0031); none for no Console.
    pub console: Option<&'a ConsoleSocket>,
    /// The environment `main` read, whose `HOME` (and `XDG_CONFIG_HOME` on
    /// Linux) say where the service manager reads its files, and whose
    /// [`PASSED_VARIABLES`] the Daemon gets.
    pub environment: &'a BTreeMap<OsString, OsString>,
    /// The launchd domain of the user's agents, such as `gui/501`.
    pub launchd_domain: Option<&'a str>,
}

/// A command a service manager is driven with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagerCommand {
    /// The program, found on `PATH`.
    pub program: &'static str,
    /// Its arguments.
    pub arguments: Vec<OsString>,
    /// Whether its failure is expected, such as removing what may not be
    /// there.
    pub may_fail: bool,
}

/// A file that defines a service to its service manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceFile {
    /// Where it goes.
    pub path: PathBuf,
    /// Its bytes.
    pub contents: Vec<u8>,
}

/// What registering a service takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicePlan {
    /// The service, as `daemon-service.json` records it.
    pub service: InstalledService,
    /// The files to write.
    pub files: Vec<ServiceFile>,
    /// The commands that register it once its files are written, after
    /// [`Unregistering::before_removal`] has removed an earlier one.
    pub register: Vec<ManagerCommand>,
}

/// What removing a service takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unregistering {
    /// The commands that stop and remove it, before its files go.
    pub before_removal: Vec<ManagerCommand>,
    /// The commands that make its service manager forget its files.
    pub after_removal: Vec<ManagerCommand>,
}

/// Why no service can be planned.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServiceError {
    /// The environment names no home folder, where the service manager
    /// reads its files.
    #[error("the environment sets no {0}, which says where the service manager reads its files")]
    NoHomeFolder(&'static str),
    /// A path cannot be written into the service's definition.
    #[error("{0} is not Unicode, which the service's definition needs")]
    NotUnicode(PathBuf),
    /// launchd needs the domain of the user's agents.
    #[error("launchd needs the domain of the user's agents")]
    NoLaunchdDomain,
    /// The OS has no socket activation, or the request gives no socket.
    #[error("{0:?} hands over a socket only with one to listen on")]
    NoSocket(ServiceManager),
}

/// The plan that registers the service `request` describes.
///
/// # Errors
///
/// When the environment does not say where the service manager reads its
/// files, a path is not Unicode, or a socket-activating manager is given
/// no socket.
pub fn plan(request: &ServiceRequest<'_>) -> Result<ServicePlan, ServiceError> {
    let name = service_name(request.manager, request.home, request.version);
    let socket = match (activates_sockets(request.manager), request.socket) {
        (true, Some(socket)) => Some(socket),
        (true, None) => return Err(ServiceError::NoSocket(request.manager)),
        (false, _) => None,
    };
    let working = unicode(request.home.root())?;
    let definition = Definition {
        name: &name,
        arguments: arguments(request)?,
        description: format!(
            "Goodfetch's Daemon of major {}, for {working}",
            major_name(request.version),
        ),
        working,
        socket: socket.map(unicode).transpose()?,
        console: socket
            .and(request.console)
            .map(|console| console.address().to_string()),
        console_held: request
            .console
            .is_some_and(|console| console.hold.is_some()),
    };
    let (files, register) = match request.manager {
        ServiceManager::Launchd => launchd(request, &definition)?,
        ServiceManager::Systemd => systemd(request, &definition)?,
        ServiceManager::TaskScheduler => task_scheduler(request, &definition),
    };
    Ok(ServicePlan {
        service: InstalledService {
            manager: request.manager,
            name: name.clone(),
            socket: socket.map(Path::to_owned),
            console: definition.console.clone(),
            executable: request.executable.to_owned(),
            files: files.iter().map(|file| file.path.clone()).collect(),
        },
        files,
        register,
    })
}

/// What every service manager's definition holds.
struct Definition<'a> {
    name: &'a str,
    arguments: Vec<String>,
    working: String,
    description: String,
    socket: Option<String>,
    /// The Console's `host:port`, which the service manager listens on too.
    console: Option<String>,
    /// Whether that port is held for the service manager to join.
    console_held: bool,
}

/// The program and arguments the service starts: the versioned copy's
/// `daemon launched --service`, with the [`PASSED_VARIABLES`] that are set.
fn arguments(request: &ServiceRequest<'_>) -> Result<Vec<String>, ServiceError> {
    let mut arguments = vec![
        unicode(request.executable)?,
        "daemon".to_owned(),
        "launched".to_owned(),
        "--service".to_owned(),
        "--copy".to_owned(),
        request.build.to_owned(),
    ];
    for variable in PASSED_VARIABLES {
        if let Some(value) = request
            .environment
            .get(OsStr::new(variable))
            .filter(|value| !value.is_empty())
        {
            let value = value
                .to_str()
                .ok_or_else(|| ServiceError::NotUnicode(PathBuf::from(value)))?;
            arguments.push("--environment".to_owned());
            arguments.push(format!("{variable}={value}"));
        }
    }
    Ok(arguments)
}

/// A service manager's files and the commands that register them.
type Planned = (Vec<ServiceFile>, Vec<ManagerCommand>);

fn launchd(
    request: &ServiceRequest<'_>,
    definition: &Definition<'_>,
) -> Result<Planned, ServiceError> {
    let domain = request
        .launchd_domain
        .ok_or(ServiceError::NoLaunchdDomain)?;
    let path = home_folder(request.environment)?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", definition.name));
    let register = vec![command(
        "launchctl",
        &[
            OsStr::new("bootstrap"),
            OsStr::new(domain),
            path.as_os_str(),
        ],
        false,
    )];
    let plist = launchd_plist(
        definition.name,
        &definition.arguments,
        &definition.working,
        definition.socket.as_deref().unwrap_or_default(),
        definition.console.as_deref(),
    );
    Ok((
        vec![ServiceFile {
            path,
            contents: plist.into_bytes(),
        }],
        register,
    ))
}

fn systemd(
    request: &ServiceRequest<'_>,
    definition: &Definition<'_>,
) -> Result<Planned, ServiceError> {
    let folder = systemd_folder(request.environment)?;
    let socket_unit = format!("{}.socket", definition.name);
    let register = vec![
        command(
            "systemctl",
            &[OsStr::new("--user"), OsStr::new("daemon-reload")],
            false,
        ),
        command(
            "systemctl",
            &[
                OsStr::new("--user"),
                OsStr::new("enable"),
                OsStr::new("--now"),
                OsStr::new(&socket_unit),
            ],
            false,
        ),
    ];
    Ok((
        vec![
            ServiceFile {
                path: folder.join(&socket_unit),
                contents: systemd_socket(
                    &definition.description,
                    definition.socket.as_deref().unwrap_or_default(),
                    definition.console.as_deref(),
                    definition.console_held,
                )
                .into_bytes(),
            },
            ServiceFile {
                path: folder.join(format!("{}.service", definition.name)),
                contents: systemd_service(
                    &definition.description,
                    &definition.arguments,
                    &definition.working,
                )
                .into_bytes(),
            },
        ],
        register,
    ))
}

fn task_scheduler(request: &ServiceRequest<'_>, definition: &Definition<'_>) -> Planned {
    let name = definition.name;
    let file_name = name.rsplit('\\').next().unwrap_or(name);
    let path = request
        .home
        .per_major(request.version)
        .join("service")
        .join(format!("{file_name}.xml"));
    let user = windows_user(request.environment);
    let xml = task_definition(
        &definition.description,
        user.as_deref(),
        &definition.arguments,
        &definition.working,
    );
    let register = vec![
        command(
            "schtasks",
            &[
                OsStr::new("/Create"),
                OsStr::new("/TN"),
                OsStr::new(name),
                OsStr::new("/XML"),
                path.as_os_str(),
                OsStr::new("/F"),
            ],
            false,
        ),
        command(
            "schtasks",
            &[OsStr::new("/Run"), OsStr::new("/TN"), OsStr::new(name)],
            false,
        ),
    ];
    (
        vec![ServiceFile {
            path,
            contents: utf16_with_bom(&xml),
        }],
        register,
    )
}

/// What removing `service` takes; `launchd_domain` is the domain of the
/// user's agents under launchd.
pub fn unregister(service: &InstalledService, launchd_domain: Option<&str>) -> Unregistering {
    let name = OsStr::new(&service.name);
    match service.manager {
        ServiceManager::Launchd => Unregistering {
            before_removal: launchd_domain
                .map(|domain| {
                    let target = format!("{domain}/{}", service.name);
                    command(
                        "launchctl",
                        &[OsStr::new("bootout"), OsStr::new(&target)],
                        true,
                    )
                })
                .into_iter()
                .collect(),
            after_removal: Vec::new(),
        },
        ServiceManager::Systemd => {
            let socket = format!("{}.socket", service.name);
            let unit = format!("{}.service", service.name);
            Unregistering {
                before_removal: vec![
                    command(
                        "systemctl",
                        &[
                            OsStr::new("--user"),
                            OsStr::new("disable"),
                            OsStr::new("--now"),
                            OsStr::new(&socket),
                        ],
                        true,
                    ),
                    command(
                        "systemctl",
                        &[OsStr::new("--user"), OsStr::new("stop"), OsStr::new(&unit)],
                        true,
                    ),
                ],
                after_removal: vec![command(
                    "systemctl",
                    &[OsStr::new("--user"), OsStr::new("daemon-reload")],
                    true,
                )],
            }
        }
        ServiceManager::TaskScheduler => Unregistering {
            before_removal: vec![
                command(
                    "schtasks",
                    &[OsStr::new("/End"), OsStr::new("/TN"), name],
                    true,
                ),
                command(
                    "schtasks",
                    &[
                        OsStr::new("/Delete"),
                        OsStr::new("/TN"),
                        name,
                        OsStr::new("/F"),
                    ],
                    true,
                ),
            ],
            after_removal: Vec::new(),
        },
    }
}

fn command(program: &'static str, arguments: &[&OsStr], may_fail: bool) -> ManagerCommand {
    ManagerCommand {
        program,
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        may_fail,
    }
}

fn unicode(path: &Path) -> Result<String, ServiceError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ServiceError::NotUnicode(path.to_owned()))
}

fn variable<'a>(environment: &'a BTreeMap<OsString, OsString>, name: &str) -> Option<&'a OsStr> {
    environment
        .get(OsStr::new(name))
        .map(OsString::as_os_str)
        .filter(|value| !value.is_empty())
}

/// The user's home folder, from `HOME`.
fn home_folder(environment: &BTreeMap<OsString, OsString>) -> Result<PathBuf, ServiceError> {
    variable(environment, "HOME")
        .map(PathBuf::from)
        .ok_or(ServiceError::NoHomeFolder("HOME"))
}

/// Where systemd's user manager reads the user's units:
/// `$XDG_CONFIG_HOME/systemd/user`, or `~/.config/systemd/user`.
fn systemd_folder(environment: &BTreeMap<OsString, OsString>) -> Result<PathBuf, ServiceError> {
    let config = match variable(environment, "XDG_CONFIG_HOME") {
        Some(config) => PathBuf::from(config),
        None => home_folder(environment)?.join(".config"),
    };
    Ok(config.join("systemd").join("user"))
}

/// The user a logon task starts for: `DOMAIN\user`.
fn windows_user(environment: &BTreeMap<OsString, OsString>) -> Option<String> {
    let user = variable(environment, "USERNAME")?.to_str()?;
    Some(
        match variable(environment, "USERDOMAIN").and_then(OsStr::to_str) {
            Some(domain) => format!("{domain}\\{user}"),
            None => user.to_owned(),
        },
    )
}

/// Escapes `text` for XML character data and attribute values.
fn xml(text: &str) -> String {
    text.chars().fold(
        String::with_capacity(text.len()),
        |mut escaped, character| {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&apos;"),
                other => escaped.push(other),
            }
            escaped
        },
    )
}

/// The launch agent: the program and its arguments, Goodfetch home as its
/// working folder, and a Unix socket, mode `0600`, that launchd listens on
/// and hands over at the first connection. No `RunAtLoad` and no
/// `KeepAlive`: the first connection starts the Daemon, and once it has
/// stopped the next one starts it again.
pub fn launchd_plist(
    label: &str,
    arguments: &[String],
    working: &str,
    socket: &str,
    console: Option<&str>,
) -> String {
    let mut plist = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n",
        "<dict>\n",
    ));
    let _ = writeln!(
        plist,
        "\t<key>Label</key>\n\t<string>{}</string>",
        xml(label)
    );
    plist.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
    for argument in arguments {
        let _ = writeln!(plist, "\t\t<string>{}</string>", xml(argument));
    }
    plist.push_str("\t</array>\n");
    let _ = writeln!(
        plist,
        "\t<key>WorkingDirectory</key>\n\t<string>{}</string>",
        xml(working)
    );
    plist.push_str("\t<key>ProcessType</key>\n\t<string>Background</string>\n");
    let _ = writeln!(
        plist,
        concat!(
            "\t<key>Sockets</key>\n",
            "\t<dict>\n",
            "\t\t<key>{}</key>\n",
            "\t\t<dict>\n",
            "\t\t\t<key>SockPathName</key>\n",
            "\t\t\t<string>{}</string>\n",
            "\t\t\t<key>SockPathMode</key>\n",
            "\t\t\t<integer>384</integer>\n",
            "\t\t</dict>"
        ),
        LAUNCHD_SOCKETS,
        xml(socket)
    );
    if let Some((host, port)) = console.and_then(|console| console.rsplit_once(':')) {
        let _ = writeln!(
            plist,
            concat!(
                "\t\t<key>{}</key>\n",
                "\t\t<dict>\n",
                "\t\t\t<key>SockNodeName</key>\n",
                "\t\t\t<string>{}</string>\n",
                "\t\t\t<key>SockServiceName</key>\n",
                "\t\t\t<string>{}</string>\n",
                "\t\t\t<key>SockType</key>\n",
                "\t\t\t<string>stream</string>\n",
                "\t\t</dict>"
            ),
            LAUNCHD_CONSOLE_SOCKET,
            xml(host.trim_start_matches('[').trim_end_matches(']')),
            xml(port)
        );
    }
    plist.push_str("\t</dict>\n");
    plist.push_str("</dict>\n</plist>\n");
    plist
}

/// Escapes `text` for a systemd unit's value: `%` starts a specifier and
/// `$` a variable in `ExecStart=`, so both are doubled.
fn systemd_value(text: &str) -> String {
    text.replace('%', "%%").replace('$', "$$")
}

/// One argument of `ExecStart=`, double-quoted with C-style escapes.
fn systemd_argument(argument: &str) -> String {
    let escaped = systemd_value(argument)
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// The user socket unit: a Unix socket, mode `0600`, and the Console's
/// `host:port` when given, passed in that order; one service for every
/// connection (`Accept=no`), removed when it stops, enabled with the
/// user's sockets. A port `held` for it takes `ReusePort=yes`, so systemd
/// can bind beside the hold; never a named port, which a second listener
/// would then share rather than being refused.
pub fn systemd_socket(
    description: &str,
    socket: &str,
    console: Option<&str>,
    held: bool,
) -> String {
    let console = console.map_or_else(String::new, |console| {
        format!("ListenStream={}\n", systemd_value(console))
    });
    let reuse_port = if held { "ReusePort=yes\n" } else { "" };
    format!(
        concat!(
            "[Unit]\n",
            "Description={}\n",
            "\n",
            "[Socket]\n",
            "ListenStream={}\n",
            "{}",
            "{}",
            "SocketMode=0600\n",
            "Accept=no\n",
            "RemoveOnStop=yes\n",
            "\n",
            "[Install]\n",
            "WantedBy=sockets.target\n",
        ),
        systemd_value(description),
        systemd_value(socket),
        console,
        reuse_port
    )
}

/// The user service the socket starts at its first connection: the
/// program and its arguments, Goodfetch home as its working folder. It is
/// never enabled itself, so only the socket starts it.
pub fn systemd_service(description: &str, arguments: &[String], working: &str) -> String {
    let command: Vec<String> = arguments
        .iter()
        .map(|argument| systemd_argument(argument))
        .collect();
    format!(
        concat!(
            "[Unit]\n",
            "Description={}\n",
            "\n",
            "[Service]\n",
            "Type=simple\n",
            "ExecStart={}\n",
            "WorkingDirectory={}\n",
        ),
        systemd_value(description),
        command.join(" "),
        systemd_value(working)
    )
}

/// One argument of a Windows command line, quoted as the C runtime parses
/// it: within quotes, backslashes before a quote and at the end double.
fn windows_argument(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_owned();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

/// The logon task: it starts the program at the user's logon, with the
/// user's own rights, never twice at once, with no time limit and on
/// battery too.
pub fn task_definition(
    description: &str,
    user: Option<&str>,
    arguments: &[String],
    working: &str,
) -> String {
    let user_id = user
        .map(|user| format!("<UserId>{}</UserId>", xml(user)))
        .unwrap_or_default();
    let (program, rest) = arguments
        .split_first()
        .map_or(("", &[][..]), |(program, rest)| (program.as_str(), rest));
    let rest: Vec<String> = rest
        .iter()
        .map(|argument| windows_argument(argument))
        .collect();
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n",
            "<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n",
            "  <RegistrationInfo>\n",
            "    <Description>{description}</Description>\n",
            "  </RegistrationInfo>\n",
            "  <Triggers>\n",
            "    <LogonTrigger>\n",
            "      <Enabled>true</Enabled>\n",
            "      {user_id}\n",
            "    </LogonTrigger>\n",
            "  </Triggers>\n",
            "  <Principals>\n",
            "    <Principal id=\"Author\">\n",
            "      {user_id}\n",
            "      <LogonType>InteractiveToken</LogonType>\n",
            "      <RunLevel>LeastPrivilege</RunLevel>\n",
            "    </Principal>\n",
            "  </Principals>\n",
            "  <Settings>\n",
            "    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n",
            "    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n",
            "    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n",
            "    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n",
            "    <Enabled>true</Enabled>\n",
            "  </Settings>\n",
            "  <Actions Context=\"Author\">\n",
            "    <Exec>\n",
            "      <Command>{program}</Command>\n",
            "      <Arguments>{arguments}</Arguments>\n",
            "      <WorkingDirectory>{working}</WorkingDirectory>\n",
            "    </Exec>\n",
            "  </Actions>\n",
            "</Task>\n",
        ),
        description = xml(description),
        user_id = user_id,
        program = xml(program),
        arguments = xml(&rest.join(" ")),
        working = xml(working),
    )
}

/// `text` in UTF-16LE after its byte-order mark, as Task Scheduler reads a
/// definition that says it is UTF-16.
fn utf16_with_bom(text: &str) -> Vec<u8> {
    [0xFEFF_u16]
        .into_iter()
        .chain(text.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(test)]
mod tests;
