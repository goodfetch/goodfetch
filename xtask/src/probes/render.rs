//! Probe case generation, TOML formatting and policy-to-probe checking
//! (ADR 0067, ticket #429).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};

use crate::check::Finding;

use super::policy::{
    Arg, CallCase, DenyBase, DenyEntry, LevelSpec, PathCase, PolicySpec, ProbeTable, SeccompRule,
    Verdict,
};

struct CallTemplate {
    args: Vec<Arg>,
    unconfined: &'static str,
}

fn num(n: i64) -> Arg {
    Arg::Number(n)
}

fn text(s: &str) -> Arg {
    Arg::Text(s.to_owned())
}

fn tpl(args: impl Into<Vec<Arg>>, unconfined: &'static str) -> CallTemplate {
    CallTemplate {
        args: args.into(),
        unconfined,
    }
}

/// Generates a probe table covering every seccomp rule, the signal rule,
/// every deny-list entry and every file rule of `level`.
pub(super) fn generate_table(policy: &PolicySpec, level: &LevelSpec) -> Result<ProbeTable> {
    let mut call = Vec::new();
    for rule in &level.seccomp {
        if rule.name == "socket"
            && let Some(families) = rule.socket_families
        {
            for family in 0..i64::from(families) {
                call.push(CallCase {
                    rule: rule.name.clone(),
                    call: rule.call.clone(),
                    args: vec![Arg::Number(family), Arg::Number(15), Arg::Number(0)],
                    unconfined: "EINVAL".to_owned(),
                    confined: verdict_outcome(rule.verdict, "EINVAL").to_owned(),
                    pid_namespace: None,
                });
            }
            continue;
        }
        let template = call_template(rule)?;
        let confined = verdict_outcome(rule.verdict, template.unconfined);
        call.push(CallCase {
            rule: rule.name.clone(),
            call: rule.call.clone(),
            args: template.args,
            unconfined: template.unconfined.to_owned(),
            confined: confined.to_owned(),
            pid_namespace: None,
        });
        if rule.name == "tgkill"
            && let Some(signal_rule) = &policy.signal_rule
        {
            call.push(signal_probe(signal_rule));
        }
    }
    if let Some(signal_rule) = &policy.signal_rule
        && !call.iter().any(|case| case.rule == *signal_rule)
    {
        call.push(signal_probe(signal_rule));
    }

    let mut path = Vec::new();
    for entry in &policy.deny_list {
        path.push(deny_probe(entry));
    }
    for rule in &policy.file_rules {
        path.push(file_rule_probe(rule)?);
    }
    Ok(ProbeTable { call, path })
}

fn signal_probe(rule: &str) -> CallCase {
    CallCase {
        rule: rule.to_owned(),
        call: "kill".to_owned(),
        args: vec![Arg::Text("parent".to_owned()), Arg::Number(0)],
        unconfined: "ok".to_owned(),
        confined: "EPERM".to_owned(),
        pid_namespace: Some("ESRCH".to_owned()),
    }
}

fn verdict_outcome(verdict: Verdict, unconfined: &str) -> &str {
    match verdict {
        Verdict::Allow => unconfined,
        Verdict::Refuse => "EPERM",
        Verdict::NotImplemented => "ENOSYS",
    }
}

fn call_template(rule: &SeccompRule) -> Result<CallTemplate> {
    if let Some(request) = rule.ioctl_request {
        let req = i64::try_from(request).with_context(|| {
            format!(
                "ioctl request {request:#x} for `{}` overflows i64",
                rule.name
            )
        })?;
        return Ok(tpl([num(-1), num(req), num(0)], "EBADF"));
    }
    let template = match rule.name.as_str() {
        "brk" | "set_tid_address" | "unshare" => tpl([num(0)], "ok"),
        "mmap" => tpl([num(0), num(0), num(3), num(34), num(-1), num(0)], "EINVAL"),
        "munmap" | "set_robust_list" | "arch_prctl" | "clone3" => tpl([num(0), num(0)], "EINVAL"),
        "mprotect" | "madvise" | "poll" | "getrandom" => tpl([num(0), num(0), num(0)], "ok"),
        "mremap" => tpl([num(0), num(0), num(0), num(0), num(0)], "EINVAL"),
        "futex" => tpl([num(0), num(1), num(0), num(0), num(0), num(0)], "EFAULT"),
        "rseq" => tpl([num(0), num(0), num(0), num(0)], "EINVAL"),
        "clone.thread" => tpl([num(0x10000), num(0), num(0), num(0), num(0)], "EINVAL"),
        "clone.process" => tpl([num(0x800), num(0), num(0), num(0), num(0)], "EINVAL"),
        "gettid" | "sched_yield" | "rt_sigreturn" | "getpid" | "getppid" | "getuid" | "geteuid"
        | "getgid" | "getegid" | "fork" | "vfork" => tpl([], "ok"),
        "sched_getaffinity" => tpl([num(0), num(0), num(0)], "EINVAL"),
        "rt_sigaction" => tpl([num(0), num(0), num(0), num(8)], "EINVAL"),
        "rt_sigprocmask" => tpl([num(0), num(0), num(0), num(8)], "ok"),
        "sigaltstack" | "clock_getres" | "gettimeofday" => tpl([num(0), num(0)], "ok"),
        "kill" => tpl([text("self"), num(0)], "ok"),
        "tgkill" => tpl([text("self"), text("self"), num(0)], "ok"),
        "read" | "write" | "readv" | "writev" | "lseek" | "getdents64" | "sendmsg" | "recvmsg"
        | "getsockname" | "getpeername" | "bind" | "connect" | "accept" => {
            tpl([num(-1), num(0), num(0)], "EBADF")
        }
        "pread64" | "pwrite64" | "accept4" => tpl([num(-1), num(0), num(0), num(0)], "EBADF"),
        "open" | "stat" | "lstat" | "statfs" | "access" | "mkdir" | "rename" | "pipe2"
        | "clock_gettime" | "nanosleep" | "getrusage" | "capget" | "io_uring_setup" => {
            tpl([num(0), num(0)], "EFAULT")
        }
        "openat" | "faccessat" | "mkdirat" | "unlinkat" => {
            tpl([num(-100), num(0), num(0)], "EFAULT")
        }
        "close" | "fsync" | "fdatasync" | "dup" => tpl([num(-1)], "EBADF"),
        "close_range" => tpl([num(2), num(1), num(0)], "EINVAL"),
        "fstat" | "fstatfs" | "ftruncate" | "shutdown" | "setns" | "listen" => {
            tpl([num(-1), num(0)], "EBADF")
        }
        "newfstatat" | "faccessat2" => tpl([num(-100), num(0), num(0), num(0)], "EFAULT"),
        "statx" => tpl([num(-100), num(0), num(0), num(0), num(0)], "EFAULT"),
        "getcwd" => tpl([num(0), num(0)], "ERANGE"),
        "readlink" => tpl([num(0), num(0), num(1)], "EFAULT"),
        "readlinkat" => tpl([num(-100), num(0), num(0), num(1)], "EFAULT"),
        "rmdir" | "unlink" | "uname" | "sysinfo" => tpl([num(0)], "EFAULT"),
        "renameat" => tpl([num(-100), num(0), num(-100), num(0)], "EFAULT"),
        "renameat2" => tpl([num(-100), num(0), num(-100), num(0), num(0)], "EFAULT"),
        "fcntl" => tpl([num(-1), num(1), num(0)], "EBADF"),
        "dup2" => tpl([num(-1), num(5)], "EBADF"),
        "dup3" => tpl([num(-1), num(5), num(0)], "EBADF"),
        "epoll_create1" => tpl([num(-1)], "EINVAL"),
        "epoll_ctl" => tpl([num(-1), num(1), num(-1), num(0)], "EFAULT"),
        "epoll_wait" => tpl([num(-1), num(0), num(1), num(0)], "EBADF"),
        "epoll_pwait" | "epoll_pwait2" => {
            tpl([num(-1), num(0), num(1), num(0), num(0), num(8)], "EBADF")
        }
        "eventfd2" => tpl([num(0), num(-1)], "EINVAL"),
        "ppoll" => tpl([num(0), num(-1), num(0), num(0), num(8)], "EINVAL"),
        "pselect6" => tpl([num(-1), num(0), num(0), num(0), num(0), num(0)], "EINVAL"),
        "clock_nanosleep" | "request_key" => tpl([num(0), num(0), num(0), num(0)], "EFAULT"),
        "prlimit64" => tpl([num(0), num(0), num(0), num(0)], "ok"),
        "prctl" => tpl([num(0xffff), num(0), num(0), num(0), num(0)], "EINVAL"),
        "exit" | "exit_group" => tpl([num(7)], "exit:7"),
        "wait4" => tpl([num(-1), num(0), num(0), num(0)], "ECHILD"),
        "execve" => tpl([num(0), num(0), num(0)], "EFAULT"),
        "socketpair.unix" => tpl([num(1), num(1), num(0), num(0)], "EFAULT"),
        "socketpair.other" => tpl([num(2), num(1), num(0), num(0)], "EFAULT"),
        "sendto" | "recvfrom" | "io_uring_enter" => {
            tpl([num(-1), num(0), num(0), num(0), num(0), num(0)], "EBADF")
        }
        "getsockopt" | "setsockopt" => tpl([num(-1), num(0), num(0), num(0), num(0)], "EBADF"),
        "ioctl.tiocsti" => tpl([num(-1), num(0x5412), num(0)], "EBADF"),
        "io_uring_register" => tpl([num(-1), num(0), num(0), num(0)], "EINVAL"),
        "ptrace" => tpl([num(16), num(0), num(0), num(0)], "ESRCH"),
        "process_vm_readv" | "process_vm_writev" => tpl(
            [text("self"), num(0), num(0), num(0), num(0), num(1)],
            "EINVAL",
        ),
        "keyctl" => tpl([num(0xffff), num(0), num(0), num(0), num(0)], "EOPNOTSUPP"),
        "add_key" => tpl([num(0), num(0), num(0), num(0), num(0)], "EFAULT"),
        "msgget" => tpl([num(0x6766), num(0)], "ENOENT"),
        "semget" | "shmget" => tpl([num(0x6766), num(0), num(0)], "ENOENT"),
        "socket" => tpl([num(0), num(15), num(0)], "EINVAL"),
        "socket.inet" => tpl([num(2), num(15), num(0)], "EINVAL"),
        "socket.inet6" => tpl([num(10), num(15), num(0)], "EINVAL"),
        "socket.unix" => tpl([num(1), num(15), num(0)], "EINVAL"),
        "socket.netlink" => tpl([num(16), num(15), num(0)], "EINVAL"),
        other => bail!("no probe template registered for seccomp rule `{other}`"),
    };
    Ok(template)
}

fn deny_probe(entry: &DenyEntry) -> PathCase {
    let path = match (entry.base, entry.path.as_str()) {
        (DenyBase::GoodfetchHome, "") => "{goodfetch_home}/settings.toml".to_owned(),
        (DenyBase::GoodfetchHome, rel) => format!("{{goodfetch_home}}/{rel}"),
        (DenyBase::Anywhere, ".env") => "{home}/project/.env.local".to_owned(),
        (DenyBase::Anywhere, ".git/hooks") => "{home}/project/.git/hooks/pre-commit".to_owned(),
        (DenyBase::Anywhere, rel) => format!("{{home}}/project/{rel}"),
        (DenyBase::Home, rel) => {
            if looks_like_file(rel) {
                format!("{{home}}/{rel}")
            } else {
                format!("{{home}}/{rel}/probe")
            }
        }
        (DenyBase::AppData, rel) => format!("{{home}}/appdata/{rel}/probe"),
        (DenyBase::LocalAppData, rel) => format!("{{home}}/localappdata/{rel}/probe"),
    };
    PathCase {
        rule: entry.rule.clone(),
        access: "read".to_owned(),
        path,
        unconfined: "ok".to_owned(),
        landlock: "EACCES".to_owned(),
        fallback: "ENOENT".to_owned(),
    }
}

fn looks_like_file(rel: &str) -> bool {
    matches!(
        rel,
        ".docker/config.json"
            | ".netrc"
            | ".git-credentials"
            | ".npmrc"
            | ".pypirc"
            | ".cargo/credentials.toml"
            | ".terraform.d/credentials.tfrc.json"
            | ".vault-token"
            | ".bashrc"
            | ".bash_profile"
            | ".zshrc"
            | ".zprofile"
            | ".profile"
    )
}

fn file_rule_probe(rule: &str) -> Result<PathCase> {
    let (access, path, landlock, fallback) = match rule {
        "files.read_bundle" => ("read", "{bundle}/index.js", "ok", "ok"),
        "files.read_runtime" => ("read", "{runtime}/lib/probe", "ok", "ok"),
        "files.write_instance" => ("write", "{instance}/written", "ok", "ok"),
        "files.read_elsewhere" => ("read", "{home}/notes.txt", "EACCES", "ENOENT"),
        "files.write_elsewhere" => ("write", "{home}/written", "EACCES", "ENOENT"),
        "files.write_bundle" => ("write", "{bundle}/written", "EACCES", "EROFS"),
        other => bail!("no probe template registered for file rule `{other}`"),
    };
    Ok(PathCase {
        rule: rule.to_owned(),
        access: access.to_owned(),
        path: path.to_owned(),
        unconfined: "ok".to_owned(),
        landlock: landlock.to_owned(),
        fallback: fallback.to_owned(),
    })
}

/// Formats `table` as a deterministic TOML file matching `goodfetch-confinement`.
pub(super) fn format_table(
    os: &str,
    level: &str,
    mechanisms: &[String],
    table: &ProbeTable,
) -> String {
    let mut out = String::new();
    let mechanisms_note = if mechanisms.is_empty() {
        String::new()
    } else {
        format!("# Mechanisms: {}.\n", mechanisms.join(", "))
    };
    let _ = writeln!(
        out,
        "# {os} probes at the `{level}` network level (ADR 0067), generated by"
    );
    let _ = writeln!(
        out,
        "# `xtask probes generate` (#429): one case per seccomp rule, deny-list"
    );
    let _ = writeln!(
        out,
        "# entry, file rule and signal rule, positive control first."
    );
    out.push_str(&mechanisms_note);
    out.push_str("\ncall = [\n");
    for case in &table.call {
        let _ = write!(
            out,
            "    {{ rule = {:?}, call = {:?}, args = [{}], unconfined = {:?}, confined = {:?}",
            case.rule,
            case.call,
            format_args(&case.args),
            case.unconfined,
            case.confined,
        );
        if let Some(pid_ns) = &case.pid_namespace {
            let _ = write!(out, ", pid_namespace = {pid_ns:?}");
        }
        out.push_str(" },\n");
    }
    out.push_str("]\n\npath = [\n");
    for case in &table.path {
        let _ = writeln!(
            out,
            "    {{ rule = {:?}, access = {:?}, path = {:?}, unconfined = {:?}, landlock = {:?}, fallback = {:?} }},",
            case.rule, case.access, case.path, case.unconfined, case.landlock, case.fallback,
        );
    }
    out.push_str("]\n");
    out
}

fn format_args(args: &[Arg]) -> String {
    args.iter()
        .map(|arg| match arg {
            Arg::Number(n) => format_number(*n),
            Arg::Text(s) => format!("{s:?}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_number(n: i64) -> String {
    if matches!(
        n,
        0x800
            | 0x5401
            | 0x5412
            | 0x5413
            | 0x541b
            | 0x5421
            | 0x5450
            | 0x5451
            | 0x6766
            | 0xffff
            | 0x10000
    ) {
        format!("{n:#x}")
    } else {
        n.to_string()
    }
}

/// Checks `table` against `policy` and `level`, returning every mismatch.
pub(super) fn check_table(
    prefix: &str,
    table: &ProbeTable,
    policy: &PolicySpec,
    level: &LevelSpec,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    check_calls(prefix, table, policy, level, &mut findings);
    check_paths(prefix, table, policy, &mut findings);
    check_coverage(prefix, table, policy, level, &mut findings);
    findings
}

fn check_calls(
    prefix: &str,
    table: &ProbeTable,
    policy: &PolicySpec,
    level: &LevelSpec,
    findings: &mut Vec<Finding>,
) {
    for case in &table.call {
        let subject = format!("{prefix}:{}", case.rule);
        if policy.signal_rule.as_deref() == Some(case.rule.as_str()) {
            if !matches!(case.call.as_str(), "kill" | "tgkill") {
                findings.push(Finding::new(
                    "other-call",
                    subject.clone(),
                    format!("signal probe uses `{}`, not kill/tgkill", case.call),
                ));
            }
            if case.confined != "EPERM" {
                findings.push(Finding::new(
                    "wrong-outcome",
                    subject.clone(),
                    format!(
                        "signal probe expects `{}` confined, not `EPERM`",
                        case.confined
                    ),
                ));
            }
            if case.pid_namespace.as_deref() != Some("ESRCH") {
                findings.push(Finding::new(
                    "wrong-outcome",
                    subject.clone(),
                    "signal probe must expect `ESRCH` under `pid_namespace`",
                ));
            }
            if case.unconfined == "EPERM" {
                findings.push(Finding::new(
                    "no-positive-control",
                    subject,
                    "signal probe gives its refusal unconfined too",
                ));
            }
            continue;
        }
        let Some(rule) = level.seccomp.iter().find(|r| r.name == case.rule) else {
            findings.push(Finding::new(
                "probe-without-rule",
                subject,
                "probe names a rule that does not exist in the policy",
            ));
            continue;
        };
        if rule.call != case.call {
            findings.push(Finding::new(
                "other-call",
                subject.clone(),
                format!(
                    "probe calls `{}` where rule names `{}`",
                    case.call, rule.call
                ),
            ));
        }
        let expected = verdict_outcome(rule.verdict, &case.unconfined);
        if case.confined != expected {
            findings.push(Finding::new(
                "wrong-outcome",
                subject.clone(),
                format!(
                    "probe expects `{}` confined where rule gives `{expected}`",
                    case.confined
                ),
            ));
        }
        if let Some(pid_ns) = &case.pid_namespace
            && pid_ns != expected
        {
            findings.push(Finding::new(
                "wrong-outcome",
                subject.clone(),
                format!("seccomp probe expects `{pid_ns}` in pid_namespace where rule gives `{expected}`"),
            ));
        }
        if rule.verdict != Verdict::Allow && case.unconfined == expected {
            findings.push(Finding::new(
                "no-positive-control",
                subject,
                "refusal probe gives its refusal unconfined too",
            ));
        }
    }
}

fn check_paths(prefix: &str, table: &ProbeTable, policy: &PolicySpec, findings: &mut Vec<Finding>) {
    for case in &table.path {
        let subject = format!("{prefix}:{}", case.rule);
        if let Some(entry) = policy.deny_list.iter().find(|e| e.rule == case.rule) {
            if !path_inside_entry(&case.path, entry) {
                findings.push(Finding::new(
                    "outside-entry",
                    subject.clone(),
                    format!(
                        "probe path `{}` lies outside deny entry `{}`",
                        case.path, entry.path
                    ),
                ));
            }
            if case.unconfined != "ok" || case.landlock != "EACCES" || case.fallback != "ENOENT" {
                findings.push(Finding::new(
                    "wrong-outcome",
                    subject,
                    "deny-list probe must expect ok unconfined, EACCES under landlock, ENOENT under fallback",
                ));
            }
        } else if !policy.file_rules.iter().any(|r| r == &case.rule) {
            findings.push(Finding::new(
                "probe-without-rule",
                subject,
                "path probe names a rule that does not exist in the policy",
            ));
        }
    }
}

fn path_inside_entry(probe_path: &str, entry: &DenyEntry) -> bool {
    match entry.base {
        DenyBase::GoodfetchHome => {
            if entry.path.is_empty() {
                probe_path.starts_with("{goodfetch_home}/")
            } else {
                let prefix = format!("{{goodfetch_home}}/{}", entry.path);
                probe_path == prefix || probe_path.starts_with(&format!("{prefix}/"))
            }
        }
        DenyBase::Home => {
            let prefix = format!("{{home}}/{}", entry.path);
            probe_path == prefix || probe_path.starts_with(&format!("{prefix}/"))
        }
        DenyBase::Anywhere | DenyBase::AppData | DenyBase::LocalAppData => {
            probe_path.contains(entry.path.as_str())
        }
    }
}

fn check_coverage(
    prefix: &str,
    table: &ProbeTable,
    policy: &PolicySpec,
    level: &LevelSpec,
    findings: &mut Vec<Finding>,
) {
    let probed: BTreeSet<&str> = table
        .call
        .iter()
        .map(|case| case.rule.as_str())
        .chain(table.path.iter().map(|case| case.rule.as_str()))
        .collect();
    for rule in &level.seccomp {
        if !probed.contains(rule.name.as_str()) {
            findings.push(Finding::new(
                "rule-without-probe",
                format!("{prefix}:{}", rule.name),
                "seccomp rule has no probe case",
            ));
        } else if rule.name == "socket"
            && let Some(families) = rule.socket_families
        {
            let count = table.call.iter().filter(|c| c.rule == "socket").count();
            if count < families as usize {
                findings.push(Finding::new(
                    "rule-without-probe",
                    format!("{prefix}:socket"),
                    format!("socket rule has {count} family cases, expected at least {families}"),
                ));
            }
        }
    }
    if let Some(signal_rule) = &policy.signal_rule
        && !probed.contains(signal_rule.as_str())
    {
        findings.push(Finding::new(
            "rule-without-probe",
            format!("{prefix}:{signal_rule}"),
            "signal rule has no probe case",
        ));
    }
    for entry in &policy.deny_list {
        if !probed.contains(entry.rule.as_str()) {
            findings.push(Finding::new(
                "rule-without-probe",
                format!("{prefix}:{}", entry.rule),
                "deny-list entry has no probe case",
            ));
        }
    }
    for rule in &policy.file_rules {
        if !probed.contains(rule.as_str()) {
            findings.push(Finding::new(
                "rule-without-probe",
                format!("{prefix}:{rule}"),
                "file rule has no probe case",
            ));
        }
    }
}
