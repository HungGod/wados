//! Passwords and PAM (libpam, called directly: five functions and a
//! conversation, no bindgen).
//!
//! - [`check`]: does this password open this account? (The lock screen's,
//!   and Settings' check of the current password before changing it.)
//! - [`change`]: a new password, the current one checked. The token's
//!   change is PAM's too (`pam_chauthtok`, the `passwd` service), but the
//!   new hash has to go into /etc/shadow, which only root writes: pam_unix
//!   called by a process that isn't root can't (Fedora's unix_update helper
//!   is root's alone). So it's `passwd`'s, the setuid-root program for
//!   exactly that, run on a pseudo-terminal of our own and answered as a
//!   person would (its current password, the new one twice), as GNOME
//!   Settings does it.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int,
    appdata_ptr: *mut c_void,
}

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        handle: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(handle: *mut PamHandle, status: c_int) -> c_int;
    fn pam_strerror(handle: *mut PamHandle, err: c_int) -> *const c_char;
}

struct Answers {
    login: CString,
    password: CString,
}

/// PAM asks; we answer: the password for a hidden prompt, the login for a
/// shown one, nothing for its messages. Answers are malloc'd: PAM frees them.
extern "C" fn converse(n: c_int, msgs: *mut *const PamMessage, out: *mut *mut PamResponse, data: *mut c_void) -> c_int {
    if n <= 0 || msgs.is_null() || out.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: data is the Answers we passed to pam_start, alive for its call.
    let answers = unsafe { &*(data as *const Answers) };
    // SAFETY: calloc of n responses; PAM frees the array and each resp.
    let replies = unsafe { libc::calloc(n as usize, std::mem::size_of::<PamResponse>()) } as *mut PamResponse;
    if replies.is_null() {
        return PAM_BUF_ERR;
    }
    for i in 0..n as usize {
        // SAFETY: Linux-PAM passes an array of n message pointers.
        let m = unsafe { &**msgs.add(i) };
        let text = match m.msg_style {
            PAM_PROMPT_ECHO_OFF => Some(&answers.password),
            PAM_PROMPT_ECHO_ON => Some(&answers.login),
            _ => None,
        };
        if let Some(t) = text {
            // SAFETY: strdup of a valid C string; replies has room for n.
            unsafe { (*replies.add(i)).resp = libc::strdup(t.as_ptr()) };
        }
    }
    // SAFETY: out is PAM's place for the array.
    unsafe { *out = replies };
    PAM_SUCCESS
}

/// Ok when `password` opens `login`'s account (PAM service `service`), else
/// PAM's word for why not.
pub fn check(service: &str, login: &str, password: &str) -> Result<(), String> {
    let service = CString::new(service).map_err(|e| e.to_string())?;
    let answers = Box::new(Answers {
        login: CString::new(login).map_err(|e| e.to_string())?,
        password: CString::new(password).map_err(|_| "a password can't hold a NUL".to_string())?,
    });
    let conv = PamConv { conv: converse, appdata_ptr: &*answers as *const Answers as *mut c_void };
    let mut handle: *mut PamHandle = std::ptr::null_mut();
    // SAFETY: valid C strings and conversation; handle is written by pam_start.
    let r = unsafe { pam_start(service.as_ptr(), answers.login.as_ptr(), &conv, &mut handle) };
    if r != PAM_SUCCESS || handle.is_null() {
        return Err(format!("pam_start failed ({r})"));
    }
    let say = |r: c_int| -> String {
        // SAFETY: pam_strerror returns a static string for a live handle.
        unsafe {
            let s = pam_strerror(handle, r);
            if s.is_null() { format!("PAM error {r}") } else { CStr::from_ptr(s).to_string_lossy().into_owned() }
        }
    };
    // SAFETY: handle is live until pam_end below.
    let mut r = unsafe { pam_authenticate(handle, 0) };
    let result = if r != PAM_SUCCESS {
        Err(say(r))
    } else {
        // SAFETY: as above.
        r = unsafe { pam_acct_mgmt(handle, 0) };
        if r == PAM_SUCCESS { Ok(()) } else { Err(say(r)) }
    };
    // SAFETY: ends the transaction started above; the conversation's data
    // (answers) outlives it.
    unsafe { pam_end(handle, r) };
    drop(answers);
    result
}

// ------------------------------------------------------------ changing it

/// Why a change didn't happen (Settings' words for each).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The current password isn't this one.
    Wrong,
    /// The new one isn't good enough (PAM's pwquality says why).
    Weak(String),
    /// Something else went wrong (what).
    Failed(String),
}

/// Changes `login`'s password from `current` to `new`: `current` checked
/// first (PAM, the `check_service`), then `passwd` does it.
pub fn change(check_service: &str, login: &str, current: &str, new: &str) -> Result<(), Refused> {
    if let Err(e) = check(check_service, login, current) {
        tracing::info!("the current password didn't open {login}'s account: {e}");
        return Err(Refused::Wrong);
    }
    let out = run_passwd(current, new).map_err(Refused::Failed)?;
    read_passwd(&out.0, out.1)
}

/// What `passwd`'s words (and whether it ended well) mean.
pub fn read_passwd(said: &str, ok: bool) -> Result<(), Refused> {
    if let Some(i) = said.find("BAD PASSWORD:") {
        let why = said[i + "BAD PASSWORD:".len()..].lines().next().unwrap_or("").trim();
        return Err(Refused::Weak(sentence(why)));
    }
    if ok && said.contains("successfully") {
        return Ok(());
    }
    // Its prompts off (no echo: the answers' newlines never came back).
    let mut said = said.to_string();
    for p in ["Current password:", "Retype new password:", "New password:"] {
        said = said.replace(p, "\n");
    }
    let last = said
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.ends_with(':') && !l.starts_with("Changing password"))
        .unwrap_or("passwd didn't say why");
    Err(Refused::Failed(last.trim_start_matches("passwd: ").to_string()))
}

/// "it is based on a dictionary word" as a sentence: "It is based on a dictionary word."
fn sentence(s: &str) -> String {
    let mut c = s.chars();
    let mut out = match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "It isn't good enough".to_string(),
    };
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// Which answer a prompt of `passwd`'s wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    Current,
    New,
    Again,
}

/// The prompt at the end of what `passwd` has said since the last answer, if it's waiting.
pub fn prompt(since: &str) -> Option<Prompt> {
    let t = since.trim_end().to_lowercase();
    if !t.ends_with("password:") {
        return None;
    }
    let line = t.lines().next_back().unwrap_or("");
    Some(if line.contains("retype") || line.contains("again") {
        Prompt::Again
    } else if line.contains("new") {
        Prompt::New
    } else {
        Prompt::Current
    })
}

/// Runs `passwd` on a pseudo-terminal, answering its prompts: everything
/// it said, and whether it ended well.
fn run_passwd(current: &str, new: &str) -> Result<(String, bool), String> {
    run_passwd_with("passwd", current, new)
}

fn run_passwd_with(program: &str, current: &str, new: &str) -> Result<(String, bool), String> {
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};

    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty writes two fds; the rest may be null.
    let r = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) };
    if r != 0 {
        return Err(format!("openpty: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: openpty gave us both, ours alone.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    // No echo: what's typed never comes back to be read (nor into an error).
    // SAFETY: a termios for a valid tty fd, read then written back.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(slave.as_raw_fd(), &mut t) == 0 {
            t.c_lflag &= !(libc::ECHO | libc::ECHONL);
            libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &t);
        }
    }
    let tty = || slave.try_clone().map(std::process::Stdio::from);
    let mut cmd = std::process::Command::new(program);
    cmd.env("LC_ALL", "C").env("LANG", "C").stdin(tty().map_err(|e| e.to_string())?);
    cmd.stdout(tty().map_err(|e| e.to_string())?).stderr(tty().map_err(|e| e.to_string())?);
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::ioctl(0, libc::TIOCSCTTY, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("passwd: {e}"))?;
    // Our copies of its terminal's end closed: once it's gone, reading ours ends.
    drop(cmd);
    drop(slave);
    let mut f = std::fs::File::from(master);
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut said, mut since) = (String::new(), String::new());
    let mut news = 0;
    let mut buf = [0u8; 1024];
    let fd = f.as_raw_fd();
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("passwd didn't finish".into());
        }
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        let n = unsafe { libc::poll(&mut p, 1, 200) };
        if n > 0 && p.revents & libc::POLLIN != 0 {
            match f.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(k) => {
                    let s = String::from_utf8_lossy(&buf[..k]);
                    said.push_str(&s);
                    since.push_str(&s);
                }
            }
        } else if n > 0 && p.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            break;
        }
        if said.contains("BAD PASSWORD:") && prompt(&since).is_some() {
            // It would only ask again: what it said is the answer.
            let _ = child.kill();
            break;
        }
        if let Some(pr) = prompt(&since) {
            let answer = match pr {
                Prompt::Current => current,
                Prompt::New | Prompt::Again => {
                    news += 1;
                    if news > 2 {
                        let _ = child.kill();
                        break;
                    }
                    new
                }
            };
            since.clear();
            f.write_all(answer.as_bytes()).and_then(|()| f.write_all(b"\n")).map_err(|e| e.to_string())?;
        }
        if let Ok(Some(_)) = child.try_wait() {
            // What's left to read (without waiting for more), then it's done.
            loop {
                let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                // SAFETY: one valid pollfd.
                if unsafe { libc::poll(&mut p, 1, 50) } <= 0 || p.revents & libc::POLLIN == 0 {
                    break;
                }
                match f.read(&mut buf) {
                    Ok(k) if k > 0 => said.push_str(&String::from_utf8_lossy(&buf[..k])),
                    _ => break,
                }
            }
            break;
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    Ok((said.replace('\r', ""), ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `passwd` that asks as Fedora's does, and takes "s3cret!pw" (the current one) to "n3w-pass!word".
    const FAKE: &str = r#"#!/bin/sh
echo "Changing password for user sam."
printf "Current password: "; read -r cur
printf "New password: "; read -r new
if [ ${#new} -lt 8 ]; then echo "BAD PASSWORD: The password is shorter than 8 characters"; printf "New password: "; read -r new; fi
printf "Retype new password: "; read -r again
[ "$cur" = "s3cret!pw" ] || { echo "passwd: Authentication token manipulation error"; exit 1; }
[ "$new" = "$again" ] || { echo "Sorry, passwords do not match."; exit 1; }
echo "passwd: all authentication tokens updated successfully."
"#;

    #[test]
    fn passwd_on_its_own_terminal() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("passwd");
        std::fs::write(&p, FAKE).unwrap();
        std::os::unix::fs::PermissionsExt::set_mode(&mut std::fs::metadata(&p).unwrap().permissions(), 0o755);
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let prog = p.to_str().unwrap();
        let (said, ok) = run_passwd_with(prog, "s3cret!pw", "n3w-pass!word").unwrap();
        assert!(ok, "{said}");
        assert_eq!(read_passwd(&said, ok), Ok(()));
        // What the typed passwords were isn't echoed back.
        assert!(!said.contains("n3w-pass!word"), "{said}");
        let (said, ok) = run_passwd_with(prog, "s3cret!pw", "short").unwrap();
        assert_eq!(read_passwd(&said, ok), Err(Refused::Weak("The password is shorter than 8 characters.".into())));
        let (said, ok) = run_passwd_with(prog, "wrong", "n3w-pass!word").unwrap();
        assert_eq!(read_passwd(&said, ok), Err(Refused::Failed("Authentication token manipulation error".into())));
    }

    #[test]
    fn passwd_s_prompts() {
        assert_eq!(prompt("Changing password for user sam.\nCurrent password: "), Some(Prompt::Current));
        assert_eq!(prompt("\nNew password: "), Some(Prompt::New));
        assert_eq!(prompt("\nRetype new password: "), Some(Prompt::Again));
        assert_eq!(prompt("Changing password for user sam.\n"), None);
        assert_eq!(prompt(""), None);
    }

    #[test]
    fn passwd_s_answers() {
        assert_eq!(
            read_passwd(
                "Current password: \nNew password: \nRetype new password: \npasswd: all authentication tokens updated successfully.\n",
                true
            ),
            Ok(())
        );
        assert_eq!(
            read_passwd(
                "New password: \nBAD PASSWORD: The password fails the dictionary check - it is based on a dictionary word\nNew password: ",
                false
            ),
            Err(Refused::Weak("The password fails the dictionary check - it is based on a dictionary word.".into()))
        );
        assert_eq!(
            read_passwd("Current password: \npasswd: Authentication token manipulation error\n", false),
            Err(Refused::Failed("Authentication token manipulation error".into()))
        );
        assert_eq!(read_passwd("", false), Err(Refused::Failed("passwd didn't say why".into())));
        assert_eq!(sentence("it is too short"), "It is too short.");
    }
}
