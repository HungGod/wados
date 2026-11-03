//! Checking a password with PAM (libpam, called directly: five functions and
//! a conversation, no bindgen). The lock screen's only use: does this
//! password open this account?

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
