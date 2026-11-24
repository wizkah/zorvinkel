//! Login of an administrator account of the device (ADR-037). The device service is
//! unprivileged and cannot ask PAM itself, so the password is checked by logging in to the
//! device's own `sshd` on loopback with the `ssh` program, which asks a helper for the password
//! (`SSH_ASKPASS`). The helper is this program: it prints the password from its environment and
//! exits, see [`answer_password_prompt`].
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
use zorvinkel_application::login::{AdminLogin, FailureLimiter, LoginOutcome};

/// Environment variable that carries the password to the helper; it exists only in the
/// environment of the one `ssh` process (and its helper) that checks a login.
pub const PASSWORD_VARIABLE: &str = "ZORVINKEL_ASKPASS_PASSWORD";
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ERROR_TEXT: u64 = 4096;
/// Groups whose members administer a Debian-based device. `root` covers uid 0 as primary group.
const ADMIN_GROUPS: [&str; 4] = ["sudo", "wheel", "admin", "root"];

/// Checks a user and password; no knowledge of administrators.
pub trait PasswordCheck {
    fn check(&mut self, user: &str, password: &str) -> PasswordResult;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordResult {
    Right,
    Wrong,
    /// The check could not be made: no `sshd` or `ssh`, password login switched off, timeout.
    Unavailable,
}

/// Login by `ssh user@127.0.0.1` with the password given to a helper program.
pub struct SshPasswordCheck {
    ssh: PathBuf,
    /// The program `ssh` starts to ask for the password (this service's own binary).
    helper: PathBuf,
    port: u16,
}
impl SshPasswordCheck {
    pub fn new(ssh: impl Into<PathBuf>, helper: impl Into<PathBuf>, port: u16) -> Self {
        Self {
            ssh: ssh.into(),
            helper: helper.into(),
            port,
        }
    }
    /// The usual `ssh` of Debian and the running program as helper.
    pub fn system() -> std::io::Result<Self> {
        Ok(Self::new("/usr/bin/ssh", std::env::current_exe()?, 22))
    }
    fn command(&self, user: &str, password: &str) -> Command {
        let mut command = Command::new(&self.ssh);
        command
            .args(["-F", "/dev/null", "-p"])
            .arg(self.port.to_string())
            .args(["-l", user])
            .args([
                "-o",
                "PreferredAuthentications=password,keyboard-interactive",
                "-o",
                "PubkeyAuthentication=no",
                "-o",
                "NumberOfPasswordPrompts=1",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "GlobalKnownHostsFile=/dev/null",
                "-o",
                "ConnectTimeout=5",
                "-o",
                "ControlPath=none",
                "-o",
                "ForwardAgent=no",
                "-o",
                "ForwardX11=no",
                "-o",
                "LogLevel=ERROR",
                "127.0.0.1",
                "exit",
                "0",
            ])
            .env_clear()
            .env("SSH_ASKPASS", &self.helper)
            // Older `ssh` asks the helper only when it has no terminal and a display is set.
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", "zorvinkel:0")
            .env(PASSWORD_VARIABLE, password)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command
    }
}
impl PasswordCheck for SshPasswordCheck {
    fn check(&mut self, user: &str, password: &str) -> PasswordResult {
        // The user name reaches the command line: refuse anything but a plain account name.
        if !zorvinkel_protocol::login::valid_user(user)
            || !zorvinkel_protocol::login::valid_password(password)
            || password.contains('\0')
        {
            return PasswordResult::Wrong;
        }
        let Ok(mut child) = self.command(user, password).spawn() else {
            return PasswordResult::Unavailable;
        };
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= CHECK_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return PasswordResult::Unavailable;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => return PasswordResult::Unavailable,
            }
        };
        if status.success() {
            return PasswordResult::Right;
        }
        let mut text = String::new();
        if let Some(stderr) = child.stderr.take() {
            let _ = stderr.take(MAX_ERROR_TEXT).read_to_string(&mut text);
        }
        classify_failure(&text)
    }
}

/// `ssh` reports a refused login as `Permission denied (methods)`, where the methods are the ones
/// the server still offered. A server that offers no password method cannot check one at all.
pub fn classify_failure(error_text: &str) -> PasswordResult {
    match error_text
        .lines()
        .find(|line| line.contains("Permission denied"))
    {
        Some(line) if line.contains("password") || line.contains("keyboard-interactive") => {
            PasswordResult::Wrong
        }
        _ => PasswordResult::Unavailable,
    }
}

/// `true` for uid 0 and for members of an administrator group, by the contents of
/// `/etc/passwd` and `/etc/group`.
pub fn is_administrator(passwd: &str, group: &str, user: &str) -> bool {
    let mut primary_group = None;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.first() == Some(&user) {
            if fields.get(2) == Some(&"0") {
                return true;
            }
            primary_group = fields.get(3).copied();
            break;
        }
    }
    let Some(primary_group) = primary_group else {
        // No such account here (for example a directory service): not decidable, so not admin.
        return false;
    };
    group.lines().any(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        let name = fields.first().copied().unwrap_or_default();
        ADMIN_GROUPS.contains(&name)
            && (fields.get(2) == Some(&primary_group)
                || fields
                    .get(3)
                    .is_some_and(|members| members.split(',').any(|member| member == user)))
    })
}

/// Login check of the device: password by `sshd`, then the administrator test, with a limit on
/// wrong attempts.
pub struct SystemAdminLogin<C> {
    check: C,
    passwd: PathBuf,
    group: PathBuf,
    limiter: FailureLimiter,
    clock: Instant,
}
impl<C: PasswordCheck> SystemAdminLogin<C> {
    pub fn new(check: C, passwd: impl Into<PathBuf>, group: impl Into<PathBuf>) -> Self {
        Self {
            check,
            passwd: passwd.into(),
            group: group.into(),
            limiter: FailureLimiter::default(),
            clock: Instant::now(),
        }
    }
    pub fn system(check: C) -> Self {
        Self::new(check, "/etc/passwd", "/etc/group")
    }
    fn read(path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}
impl<C: PasswordCheck> AdminLogin for SystemAdminLogin<C> {
    fn verify(&mut self, user: &str, password: &str) -> LoginOutcome {
        let now = self.clock.elapsed().as_secs();
        if !self.limiter.allows(now) {
            return LoginOutcome::Throttled;
        }
        // The password travels in a copy that is wiped when the check is over.
        let password = Zeroizing::new(password.to_owned());
        match self.check.check(user, &password) {
            PasswordResult::Unavailable => LoginOutcome::Unavailable,
            PasswordResult::Wrong => {
                self.limiter.record_failure(now);
                LoginOutcome::Rejected
            }
            PasswordResult::Right => {
                let (Some(passwd), Some(group)) =
                    (Self::read(&self.passwd), Self::read(&self.group))
                else {
                    return LoginOutcome::Unavailable;
                };
                if is_administrator(&passwd, &group, user) {
                    LoginOutcome::Accepted
                } else {
                    LoginOutcome::NotAdministrator
                }
            }
        }
    }
}

/// Entry of the helper program: when `ssh` asks for the password, the service binary is started
/// with the prompt as its only argument and the variable set. Returns the text to print.
pub fn answer_password_prompt(arguments: &[String]) -> Option<Zeroizing<String>> {
    if arguments.len() > 1 {
        return None;
    }
    std::env::var(PASSWORD_VARIABLE).ok().map(Zeroizing::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/bash
pi:x:1000:1000:,,,:/home/pi:/bin/bash
guest:x:1001:1001::/home/guest:/bin/bash
ops:x:1002:27::/home/ops:/bin/bash
";
    const GROUP: &str = "\
root:x:0:
sudo:x:27:pi
users:x:100:guest
pia:x:1000:
";

    #[test]
    fn administrators_are_root_and_members_of_the_admin_groups() {
        assert!(is_administrator(PASSWD, GROUP, "root"));
        assert!(is_administrator(PASSWD, GROUP, "pi"), "listed in sudo");
        assert!(
            is_administrator(PASSWD, GROUP, "ops"),
            "sudo is its primary group"
        );
        assert!(!is_administrator(PASSWD, GROUP, "guest"));
        assert!(
            !is_administrator(PASSWD, GROUP, "nobody"),
            "unknown account"
        );
        assert!(!is_administrator("", "", "pi"));
    }
    #[test]
    fn a_refused_login_is_told_apart_from_a_server_that_cannot_check_passwords() {
        assert_eq!(
            classify_failure("pi@127.0.0.1: Permission denied (publickey,password).\n"),
            PasswordResult::Wrong
        );
        assert_eq!(
            classify_failure("Permission denied (publickey,keyboard-interactive)."),
            PasswordResult::Wrong
        );
        assert_eq!(
            classify_failure("pi@127.0.0.1: Permission denied (publickey)."),
            PasswordResult::Unavailable
        );
        assert_eq!(
            classify_failure("ssh: connect to host 127.0.0.1 port 22: Connection refused"),
            PasswordResult::Unavailable
        );
        assert_eq!(classify_failure(""), PasswordResult::Unavailable);
    }

    struct Scripted {
        result: PasswordResult,
        calls: usize,
    }
    impl PasswordCheck for Scripted {
        fn check(&mut self, _: &str, _: &str) -> PasswordResult {
            self.calls += 1;
            self.result
        }
    }
    fn files(name: &str) -> (PathBuf, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("zorvinkel-login-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("passwd"), PASSWD).unwrap();
        std::fs::write(directory.join("group"), GROUP).unwrap();
        (directory.join("passwd"), directory.join("group"))
    }
    fn login(name: &str, result: PasswordResult) -> SystemAdminLogin<Scripted> {
        let (passwd, group) = files(name);
        SystemAdminLogin::new(Scripted { result, calls: 0 }, passwd, group)
    }

    #[test]
    fn a_right_password_of_an_administrator_is_accepted() {
        let mut login = login("accept", PasswordResult::Right);
        assert_eq!(login.verify("pi", "secret"), LoginOutcome::Accepted);
        assert_eq!(
            login.verify("guest", "secret"),
            LoginOutcome::NotAdministrator
        );
    }
    #[test]
    fn wrong_passwords_are_counted_and_stop_after_the_limit() {
        let mut login = login("limit", PasswordResult::Wrong);
        for _ in 0..zorvinkel_application::login::MAX_FAILURES {
            assert_eq!(login.verify("pi", "wrong"), LoginOutcome::Rejected);
        }
        assert_eq!(login.verify("pi", "wrong"), LoginOutcome::Throttled);
        assert_eq!(
            login.check.calls,
            zorvinkel_application::login::MAX_FAILURES,
            "no password is tried while throttled"
        );
    }
    #[test]
    fn an_unavailable_check_is_no_wrong_attempt() {
        let mut login = login("unavailable", PasswordResult::Unavailable);
        for _ in 0..zorvinkel_application::login::MAX_FAILURES + 2 {
            assert_eq!(login.verify("pi", "x"), LoginOutcome::Unavailable);
        }
    }
    #[test]
    fn missing_account_files_make_a_right_password_unavailable() {
        let mut login = SystemAdminLogin::new(
            Scripted {
                result: PasswordResult::Right,
                calls: 0,
            },
            "/nonexistent/passwd",
            "/nonexistent/group",
        );
        assert_eq!(login.verify("pi", "secret"), LoginOutcome::Unavailable);
    }
    #[test]
    fn odd_user_names_never_reach_the_command_line() {
        let mut check = SshPasswordCheck::new("/nonexistent/ssh", "/nonexistent/helper", 22);
        for user in ["-oProxyCommand=x", "a b", "", "Ünï"] {
            assert_eq!(
                check.check(user, "secret"),
                PasswordResult::Wrong,
                "{user:?}"
            );
        }
        assert_eq!(
            check.check("pi", "secret"),
            PasswordResult::Unavailable,
            "a missing program is no wrong password"
        );
    }

    /// A stand-in for `ssh`: succeeds when the helper prints "right", fails like `ssh` otherwise.
    #[cfg(unix)]
    #[test]
    fn the_password_reaches_the_helper_and_the_exit_status_decides() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("zorvinkel-fake-ssh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let helper = directory.join("helper");
        std::fs::write(
            &helper,
            format!("#!/bin/sh\nprintf '%s' \"${PASSWORD_VARIABLE}\"\n"),
        )
        .unwrap();
        let ssh = directory.join("ssh");
        std::fs::write(
            &ssh,
            "#!/bin/sh\nif [ \"$($SSH_ASKPASS)\" = right ]; then exit 0; fi\necho 'Permission denied (publickey,password).' >&2\nexit 255\n",
        )
        .unwrap();
        for path in [&helper, &ssh] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut check = SshPasswordCheck::new(&ssh, &helper, 22);
        assert_eq!(check.check("pi", "right"), PasswordResult::Right);
        assert_eq!(check.check("pi", "wrong"), PasswordResult::Wrong);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
