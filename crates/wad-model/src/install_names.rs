//! What the installer asks for, and what it refuses (the spike's
//! `apps/installer/names.ts` and `shell/firstboot/helpers.ts`): the same
//! rules in the installer's window and in wadd, which has the last word.

/// Names a machine can't have: `<name>.wadspaces.com` is its public address.
pub const RESERVED_HOSTS: [&str; 7] = ["www", "api", "mail", "docs", "status", "app", "admin"];
/// Accounts the system keeps.
pub const RESERVED_USERS: [&str; 4] = ["root", "admin", "wad", "wadspaces-clip"];
pub const NAME_MAX: usize = 30;
pub const PASSWORD_MIN: usize = 8;

/// Why this computer can't have this name, if it can't.
pub fn computer_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Give this computer a name".into());
    }
    if name.chars().count() > NAME_MAX {
        return Some(format!("At most {NAME_MAX} characters"));
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Some("Lowercase letters, digits and hyphens only".into());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Some("It can't start or end with a hyphen".into());
    }
    if RESERVED_HOSTS.contains(&name) {
        return Some(format!("“{name}” is kept for wadspaces.com"));
    }
    None
}

/// Why this can't be the account's name, if it can't.
pub fn username_problem(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let first = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    let rest = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !first || !rest || name.len() > 32 {
        return Some("Lowercase letters, digits, - and _, starting with a letter".into());
    }
    if RESERVED_USERS.contains(&name) {
        return Some(format!("“{name}” is taken by the system"));
    }
    None
}

/// Why this can't be a password (the account's, or the disk's).
pub fn password_problem(password: &str) -> Option<String> {
    (password.chars().count() < PASSWORD_MIN).then(|| format!("At least {PASSWORD_MIN} characters"))
}

/// How a password rates: 0 (none) to 3, and the meter's word.
pub fn strength(password: &str) -> (u8, &'static str) {
    if password.is_empty() {
        return (0, "");
    }
    let has = |f: fn(char) -> bool| password.chars().any(f);
    let kinds = [
        has(|c| c.is_ascii_lowercase()),
        has(|c| c.is_ascii_uppercase()),
        has(|c| c.is_ascii_digit()),
        has(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|k| **k)
    .count();
    let len = password.chars().count();
    if len < PASSWORD_MIN {
        (1, "Too short")
    } else if len >= 14 || (len >= 10 && kinds >= 3) {
        (3, "Strong")
    } else if len >= 10 || kinds >= 2 {
        (2, "OK")
    } else {
        (1, "Weak")
    }
}

/// The name as it's shown: Studio for `studio`.
pub fn shown_name(name: &str) -> String {
    let mut c = name.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_computer_name_is_a_dns_label_that_isnt_kept() {
        assert_eq!(computer_name_problem("studio"), None);
        assert_eq!(computer_name_problem("desk-2"), None);
        assert_eq!(computer_name_problem("").as_deref(), Some("Give this computer a name"));
        assert_eq!(computer_name_problem(&"a".repeat(31)).as_deref(), Some("At most 30 characters"));
        assert_eq!(computer_name_problem("Desk").as_deref(), Some("Lowercase letters, digits and hyphens only"));
        assert_eq!(computer_name_problem("my.desk").as_deref(), Some("Lowercase letters, digits and hyphens only"));
        assert_eq!(computer_name_problem("-desk").as_deref(), Some("It can't start or end with a hyphen"));
        assert_eq!(computer_name_problem("desk-").as_deref(), Some("It can't start or end with a hyphen"));
        for kept in RESERVED_HOSTS {
            assert_eq!(computer_name_problem(kept), Some(format!("“{kept}” is kept for wadspaces.com")));
        }
    }

    #[test]
    fn an_account_is_a_unix_name_the_system_doesnt_use() {
        assert_eq!(username_problem("sam"), None);
        assert_eq!(username_problem("_sam-2"), None);
        let rule = Some("Lowercase letters, digits, - and _, starting with a letter".to_string());
        assert_eq!(username_problem(""), rule);
        assert_eq!(username_problem("2sam"), rule);
        assert_eq!(username_problem("Sam"), rule);
        assert_eq!(username_problem("sam;rm"), rule);
        assert_eq!(username_problem(&"a".repeat(33)), rule);
        assert_eq!(username_problem("root").as_deref(), Some("“root” is taken by the system"));
    }

    #[test]
    fn the_meter_reads_as_the_spikes() {
        assert_eq!(strength(""), (0, ""));
        assert_eq!(strength("short"), (1, "Too short"));
        assert_eq!(strength("abcdefgh"), (1, "Weak"));
        assert_eq!(strength("abcdefg1"), (2, "OK"));
        assert_eq!(strength("abcdefghij"), (2, "OK"));
        assert_eq!(strength("abcdefgh1!"), (3, "Strong"));
        assert_eq!(strength("abcdefghijklmn"), (3, "Strong"));
        assert_eq!(password_problem("1234567").as_deref(), Some("At least 8 characters"));
        assert_eq!(password_problem("12345678"), None);
    }

    #[test]
    fn a_name_is_shown_capitalised() {
        assert_eq!(shown_name("studio"), "Studio");
        assert_eq!(shown_name(""), "");
    }
}
