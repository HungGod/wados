//! `wados-pkgs`: wad-gen's packages, chains and sets for scripts and CI
//! (`images/build.sh`, `.github/workflows/packages.yml`). Each command is a
//! thin call into wad-gen, so neither keeps a copy of its order, needs,
//! names, labels, keys or Containerfiles. Text in, text out: no podman, no
//! network.
//!
//! ```text
//! wados-pkgs packages [<id>…] [--wave <n>]
//! wados-pkgs containerfile <id> [--base <ref>] [--need <id>=<ref>]… [--token]
//! wados-pkgs labels <id> --meta <meta.json> [--abi <abi>] [--built-here]
//! wados-pkgs local-ref <id> <image id>
//! wados-pkgs remote-ref <id> <abi>
//! wados-pkgs chain-ref <base id> [<id>=<ref>]…
//! wados-pkgs chain-step <from> <from id> <id>=<ref>…
//! wados-pkgs set <base id> <id>…
//! wados-pkgs set-containerfile <key> <from> <base id> [<id>=<ref>]…
//! wados-pkgs index <old index.json> <abi> <base digest> [<result.json>…]
//! ```
//!
//! [`USAGE`] says what each prints. Exit 2: not how it's called; 1: wad-gen
//! said no (a package it doesn't know, a need missing, a file unreadable).

use std::io::Write;
use std::process::ExitCode;

use serde_json::{Value, json};
use wad_gen::index::{Index, Pushed};
use wad_gen::packages as p;
use wad_gen::{BASE_IMAGE, SetBuild, labels, sets};

const USAGE: &str = "\
usage: wados-pkgs <command> …  (wad-gen's packages, chains and sets)

  packages [<id>…] [--wave <n>]
      JSON, in WadOS's order: every package CI builds, or <id>s and what
      they need; each {id, name, needs, wave, meta}. needs: everything
      under it, bottom first; wave: CI's (each wave needs only earlier
      ones); meta: its meta.json's path in it. --wave: that wave's only.
  containerfile <id> [--base <ref>] [--need <id>=<ref>]… [--token]
      the Containerfile that captures it on the base (default
      localhost/wados-base:44), on every package in its needs;
      --token: a github_token secret is at hand.
  labels <id> --meta <meta.json> [--abi <abi>] [--built-here]
      the LABEL line it gets after its capture: the version and content
      its meta.json says, and the base's ABI (default: meta.json's).
  local-ref <id> <image id>         its name on a machine, by image id
  remote-ref <id> <abi>             where CI publishes it
  chain-ref <base id> [<id>=<ref>]…
      the chain image of those packages (bottom first) on the base
  chain-step <from> <from id> <id>=<ref>…
      a chain step's Containerfile: the last package copied onto <from>
      (the base or the chain under it), labelled with the chain so far
  set <base id> <id>…
      JSON: the set of those packages and what they need, with no commands
      of its own: {packages, key, image}
  set-containerfile <key> <from> <base id> [<id>=<ref>]…
      that set's Containerfile, FROM its top chain (or the base)
  index <old index.json> <abi> <base digest> [<result.json>…]
      CI's index.json with this run's pushed packages in (an old index
      that's missing or unreadable starts a new one)
";

/// Why a command didn't print.
#[derive(Debug, PartialEq, Eq)]
enum Fail {
    /// Not how it's called (exit 2, with the usage).
    Usage(String),
    /// wad-gen, or a file, said no (exit 1).
    Error(String),
}

fn usage(why: impl Into<String>) -> Fail {
    Fail::Usage(why.into())
}

fn error(e: impl ToString) -> Fail {
    Fail::Error(e.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            // A closed pipe (`| head`) isn't worth a panic.
            let _ = std::io::stdout().write_all(out.as_bytes());
            ExitCode::SUCCESS
        }
        Err(Fail::Usage(why)) => {
            eprintln!("wados-pkgs: {why}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Fail::Error(e)) => {
            eprintln!("wados-pkgs: {e}");
            ExitCode::FAILURE
        }
    }
}

/// One command: what it prints.
fn run(args: &[String]) -> Result<String, Fail> {
    let Some((cmd, rest)) = args.split_first() else { return Err(usage("which command?")) };
    match cmd.as_str() {
        "packages" => packages(rest),
        "containerfile" => containerfile(rest),
        "labels" => package_labels(rest),
        "local-ref" => match rest {
            [id, image_id] => Ok(line(p::local_ref(id, image_id))),
            _ => Err(usage("local-ref <id> <image id>")),
        },
        "remote-ref" => match rest {
            [id, abi] => Ok(line(p::remote_ref(id, abi))),
            _ => Err(usage("remote-ref <id> <abi>")),
        },
        "chain-ref" => match rest {
            [base_id, chain @ ..] => Ok(line(p::chain_ref(base_id, &pairs(chain)?))),
            _ => Err(usage("chain-ref <base id> [<id>=<ref>]…")),
        },
        "chain-step" => match rest {
            [from, from_id, chain @ ..] if !chain.is_empty() => {
                let chain = pairs(chain)?;
                let (_, pkg_ref) = chain.last().expect("one at least");
                Ok(p::chain_step(from, from_id, pkg_ref, &chain))
            }
            _ => Err(usage("chain-step <from> <from id> <id>=<ref>…")),
        },
        "set" => set(rest),
        "set-containerfile" => match rest {
            [key, from, base_id, stacked @ ..] => {
                let packages = pairs(stacked)?;
                let build = SetBuild { key, from, base_id, packages: &packages, commands: &[] };
                build.containerfile().map(|f| f.containerfile).map_err(error)
            }
            _ => Err(usage("set-containerfile <key> <from> <base id> [<id>=<ref>]…")),
        },
        "index" => index(rest),
        "-h" | "--help" | "help" => Ok(USAGE.to_string()),
        other => Err(usage(format!("no command {other:?}"))),
    }
}

fn line(s: String) -> String {
    s + "\n"
}

/// `<id>=<ref>` arguments: (id, image ref), as wad-gen takes them.
fn pairs(args: &[String]) -> Result<Vec<(String, String)>, Fail> {
    args.iter()
        .map(|a| match a.split_once('=') {
            Some((id, r)) if !id.is_empty() && !r.is_empty() => Ok((id.to_string(), r.to_string())),
            _ => Err(usage(format!("{a:?} isn't <id>=<image ref>"))),
        })
        .collect()
}

/// Each of `ids` is a package wad-gen can build.
fn known(ids: &[String]) -> Result<(), Fail> {
    match ids.iter().find(|id| p::source(id).is_none()) {
        Some(bad) => Err(error(p::Error::Unknown(bad.clone()))),
        None => Ok(()),
    }
}

fn packages(args: &[String]) -> Result<String, Fail> {
    let (mut ids, mut only) = (Vec::new(), None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--wave" => {
                let n = it.next().and_then(|n| n.parse::<usize>().ok());
                only = Some(n.ok_or_else(|| usage("--wave takes a number"))?);
            }
            o if o.starts_with('-') => return Err(usage(format!("packages: no option {o}"))),
            id => ids.push(id.to_string()),
        }
    }
    known(&ids)?;
    let ids = if ids.is_empty() { p::all() } else { ids };
    let waves = p::waves(&ids);
    let mut out = Vec::new();
    for id in p::closure(&ids) {
        let wave = waves
            .iter()
            .position(|w| w.contains(&id))
            .ok_or_else(|| error(format!("{id} can't go in a wave: what it needs never comes")))?;
        if only.is_none_or(|n| n == wave) {
            let needs = p::closure(&p::needs(&id));
            out.push(json!({"id": id, "name": p::name(&id), "needs": needs, "wave": wave, "meta": p::meta_path(&id)}));
        }
    }
    Ok(line(Value::Array(out).to_string()))
}

fn containerfile(args: &[String]) -> Result<String, Fail> {
    let Some((id, rest)) = args.split_first().filter(|(id, _)| !id.starts_with('-')) else {
        return Err(usage("containerfile <id> [--base <ref>] [--need <id>=<ref>]… [--token]"));
    };
    let (mut base, mut needs, mut token) = (BASE_IMAGE.to_string(), Vec::new(), false);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base" => base = it.next().ok_or_else(|| usage("--base takes an image"))?.clone(),
            "--need" => {
                needs.extend(pairs(std::slice::from_ref(it.next().ok_or_else(|| usage("--need takes <id>=<ref>"))?))?)
            }
            "--token" => token = true,
            o => return Err(usage(format!("containerfile: no option {o}"))),
        }
    }
    p::package_containerfile(id, &base, &needs, token).map_err(error)
}

/// A Containerfile's double-quoted value: JSON's escapes, and `$` kept
/// from being read as a variable.
fn label_value(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default().replace('$', "\\$")
}

fn package_labels(args: &[String]) -> Result<String, Fail> {
    let Some((id, rest)) = args.split_first().filter(|(id, _)| !id.starts_with('-')) else {
        return Err(usage("labels <id> --meta <meta.json> [--abi <abi>] [--built-here]"));
    };
    let (mut meta, mut abi, mut here) = (None, None, false);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--meta" => meta = Some(it.next().ok_or_else(|| usage("--meta takes a file"))?.clone()),
            "--abi" => abi = Some(it.next().ok_or_else(|| usage("--abi takes the base's ABI"))?.clone()),
            "--built-here" => here = true,
            o => return Err(usage(format!("labels: no option {o}"))),
        }
    }
    let file = meta.ok_or_else(|| usage("labels needs --meta <meta.json>"))?;
    let text = std::fs::read_to_string(&file).map_err(|e| error(format!("{file}: {e}")))?;
    let m: Value = serde_json::from_str(&text).map_err(|e| error(format!("{file}: {e}")))?;
    let field = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or_default();
    if field("id") != id {
        return Err(error(format!("{file} is the {:?} package's, not {id}'s", field("id"))));
    }
    let content = field("content");
    if content.is_empty() {
        return Err(error(format!("{file} has no content hash")));
    }
    let abi = match abi {
        Some(a) if !field("abi").is_empty() && field("abi") != a => {
            return Err(error(format!("{id} was captured on a base for {}, not {a}", field("abi"))));
        }
        Some(a) => a,
        None if !field("abi").is_empty() => field("abi").to_string(),
        None => return Err(error(format!("{file} doesn't say the base's ABI: give --abi"))),
    };
    let mut out = vec![
        (labels::PKG_VERSION, field("version").to_string()),
        (labels::PKG_ABI, abi),
        (labels::PKG_CONTENT, content.to_string()),
    ];
    if here {
        out.push((labels::PKG_BUILT_HERE, "true".into()));
    }
    let words: Vec<String> = out.iter().map(|(k, v)| format!("{k}={}", label_value(v))).collect();
    Ok(line(format!("LABEL {}", words.join(" "))))
}

fn set(args: &[String]) -> Result<String, Fail> {
    let Some((base_id, ids)) = args.split_first().filter(|(_, ids)| !ids.is_empty()) else {
        return Err(usage("set <base id> <id>…"));
    };
    known(ids)?;
    let packages = p::closure(ids);
    let key = sets::set_key(&packages, base_id, &[]);
    Ok(line(json!({"packages": packages, "image": sets::set_image(&key), "key": key}).to_string()))
}

fn index(args: &[String]) -> Result<String, Fail> {
    let [old, abi, base, results @ ..] = args else {
        return Err(usage("index <old index.json> <abi> <base digest> [<result.json>…]"));
    };
    // The first run has none: no file, or `{}` (CI writes that for it).
    let old = match std::fs::read(old) {
        Ok(bytes) if serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| v == json!({})) => None,
        Ok(bytes) => match serde_json::from_slice::<Index>(&bytes) {
            Ok(i) => Some(i),
            Err(e) => {
                eprintln!("wados-pkgs: {old} isn't an index ({e}): starting a new one");
                None
            }
        },
        Err(_) => None,
    };
    let pushed = results
        .iter()
        .map(|f| {
            let bytes = std::fs::read(f).map_err(|e| error(format!("{f}: {e}")))?;
            serde_json::from_slice::<Pushed>(&bytes).map_err(|e| error(format!("{f}: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let index = Index::merge(old, abi, base, &pushed);
    serde_json::to_string_pretty(&index).map(line).map_err(error)
}

#[cfg(test)]
mod tests;
