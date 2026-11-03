// A wadspace's commands of its own (RUN, COPY), as the Builder names them:
// RUN_1, RUN_2, COPY_1, numbered by kind as they're added. Typed exactly as
// a Containerfile has them: "RUN …" or "COPY …" at the start says which.

use crate::check::MAX_COMMAND;
use crate::types::{BuildCommand, CommandKind};

/// Why a typed command can't go in (its words are the Builder's).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("Type the command to run")]
    NoCommand,
    #[error("Type where from, then where to")]
    NoPaths,
    #[error("COPY needs where from and where to: COPY ~/notes /home/you/notes")]
    OnePath,
    #[error("A command is one line")]
    Newline,
    #[error("That command is too long")]
    TooLong,
}

/// The number at the end of a command's name ('RUN_3' → 3; none → 0).
fn number_of(name: &str) -> u64 {
    let digits = name.len() - name.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (head, n) = name.split_at(name.len() - digits);
    if digits > 0 && head.ends_with('_') { n.parse().unwrap_or(0) } else { 0 }
}

/// The name the next `kind` command takes among `existing` (after the highest: a gap isn't filled).
pub fn next_command_name(existing: &[BuildCommand], kind: CommandKind) -> String {
    let top = existing.iter().filter(|c| c.kind == kind).map(|c| number_of(&c.name)).max().unwrap_or(0);
    format!("{}_{}", kind.word(), top + 1)
}

/// Commands made from lines (kind and text), named in order.
pub fn named_commands(lines: &[(CommandKind, &str)], id: impl Fn(usize) -> String) -> Vec<BuildCommand> {
    let mut out: Vec<BuildCommand> = vec![];
    for (i, (kind, text)) in lines.iter().enumerate() {
        let name = next_command_name(&out, *kind);
        out.push(BuildCommand { id: id(i), name, kind: *kind, text: text.to_string() });
    }
    out
}

/// What was typed, as a command: "RUN …" or "COPY …" at its start says which (else `kind`); its text, trimmed. One line, and COPY needs where from and where to.
pub fn parse_command(kind: CommandKind, typed: &str) -> Result<(CommandKind, String), CommandError> {
    let mut kind = kind;
    let mut text = typed.trim();
    for (word, k) in [("run", CommandKind::Run), ("copy", CommandKind::Copy)] {
        let Some(head) = text.get(..word.len()) else { continue };
        let rest = &text[word.len()..];
        if head.eq_ignore_ascii_case(word) && (rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            kind = k;
            text = rest.trim();
            break;
        }
    }
    if text.contains(['\n', '\r']) {
        return Err(CommandError::Newline);
    }
    if text.is_empty() {
        return Err(if kind == CommandKind::Run { CommandError::NoCommand } else { CommandError::NoPaths });
    }
    if text.len() > MAX_COMMAND {
        return Err(CommandError::TooLong);
    }
    if kind == CommandKind::Copy && text.split_whitespace().count() < 2 {
        return Err(CommandError::OnePath);
    }
    Ok((kind, text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(name: &str, kind: CommandKind) -> BuildCommand {
        BuildCommand { id: name.to_lowercase(), name: name.into(), kind, text: "x".into() }
    }

    #[test]
    fn numbered_by_kind_after_the_highest() {
        assert_eq!(next_command_name(&[], CommandKind::Run), "RUN_1");
        let have = [cmd("COPY_2", CommandKind::Copy), cmd("RUN_2", CommandKind::Run)];
        assert_eq!(next_command_name(&have, CommandKind::Run), "RUN_3");
        assert_eq!(next_command_name(&have, CommandKind::Copy), "COPY_3");
        assert_eq!(next_command_name(&[cmd("RUN_custom_3", CommandKind::Run)], CommandKind::Run), "RUN_4");
        let named =
            named_commands(&[(CommandKind::Copy, "a b"), (CommandKind::Run, "x"), (CommandKind::Copy, "c d")], |i| {
                format!("c{i}")
            });
        assert_eq!(named.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["COPY_1", "RUN_1", "COPY_2"]);
    }

    #[test]
    fn run_or_copy_as_typed() {
        use CommandKind::{Copy, Run};
        assert_eq!(parse_command(Run, "   "), Err(CommandError::NoCommand));
        assert_eq!(parse_command(Copy, "   "), Err(CommandError::NoPaths));
        assert_eq!(parse_command(Copy, "~/notes"), Err(CommandError::OnePath));
        assert_eq!(parse_command(Copy, "cargo install ripgrep"), Ok((Copy, "cargo install ripgrep".into())));
        assert_eq!(parse_command(Run, "RUN cargo install ripgrep"), Ok((Run, "cargo install ripgrep".into())));
        assert_eq!(parse_command(Run, "copy ~/a /b"), Ok((Copy, "~/a /b".into())));
        assert_eq!(parse_command(Run, "RUN"), Err(CommandError::NoCommand));
        assert_eq!(parse_command(Run, "runner --fast"), Ok((Run, "runner --fast".into())));
        assert_eq!(parse_command(Run, "echo a\nrm -rf /"), Err(CommandError::Newline));
        assert_eq!(parse_command(Run, "echo a\n"), Ok((Run, "echo a".into())));
        assert_eq!(parse_command(Run, &"x".repeat(MAX_COMMAND + 1)), Err(CommandError::TooLong));
    }
}
