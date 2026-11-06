//! A pretend Claude Code on the fake machine: real turns, MCP over a real
//! socket, and no model.

use std::sync::Mutex;

use serde_json::json;
use wad_model::{AppId, Ext};

use super::*;

fn line(v: Value) -> String {
    v.to_string()
}

fn done(text: &str) -> String {
    line(json!({ "type": "result", "subtype": "success", "is_error": false, "result": text }))
}

/// One script per turn: the lines it prints.
#[derive(Default)]
struct Fake {
    turns: Mutex<Vec<Vec<String>>>,
    seen: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl Runner for Fake {
    async fn run(&self, _: &str, args: &[String], on_line: &mut OnLine<'_>) -> Result<(), String> {
        self.seen.lock().unwrap().push(args.to_vec());
        let turn = {
            let mut t = self.turns.lock().unwrap();
            if t.is_empty() { vec![done("ok")] } else { t.remove(0) }
        };
        for l in turn {
            on_line(&l);
        }
        Ok(())
    }
    async fn interrupt(&self, _: &str) {}
}

#[derive(Default)]
struct Boxes {
    log: Mutex<Vec<String>>,
}

#[async_trait]
impl Hands for Boxes {
    async fn open(&self, _: &str, _: &str) -> Result<String, String> {
        self.log.lock().unwrap().push("open".into());
        Ok("box".into())
    }
    async fn update(&self, _: &str, _: &str, path: Option<&str>, _: Option<&str>) -> Result<(), String> {
        self.log.lock().unwrap().push(format!("update {path:?}"));
        Ok(())
    }
    async fn close(&self, _: &str) {
        self.log.lock().unwrap().push("close".into());
    }
    async fn ensure_up(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

struct Up {
    _dir: tempfile::TempDir,
    m: Arc<MachineAgents>,
    fake: Arc<Fake>,
    boxes: Arc<Boxes>,
    ws: String,
    sockets: PathBuf,
}

fn up(switch: bool) -> Up {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let bus = Bus::new(wad_proto::MachineInfo {
        id: "m".into(),
        name: "M".into(),
        version: "0".into(),
        profile: wad_proto::Profile::System,
    });
    let docs = Arc::new(Docs::open(crate::store::Store::new(&state).unwrap(), bus.clone(), "m").unwrap());
    let ws = wad_model::starter::STARTER_ID.to_string();
    docs.apply(wad_model::Op::SetAgents { ws: ws.clone(), on: switch }).unwrap();
    let fake = Arc::new(Fake::default());
    let boxes = Arc::new(Boxes::default());
    let sockets = dir.path().join("s");
    let m = MachineAgents::new(&state, &sockets, fake.clone(), boxes.clone(), docs, bus);
    let _ = Ext::new(AppId::Vscode, "x");
    Up { _dir: dir, m, fake, boxes, ws, sockets }
}

async fn settled(m: &MachineAgents, id: &str) -> Agent {
    for _ in 0..200 {
        let a = m.state().agents.into_iter().find(|a| a.id == id).unwrap();
        if a.status != AgentStatus::Working {
            return a;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("it didn't settle");
}

#[test]
fn a_turn_cant_push_until_the_person_says_so() {
    let a = turn_args("do it", None, false);
    let at = a.iter().position(|x| x == "--disallowedTools").unwrap();
    assert!(a[at + 1].contains("git push") && a[at + 1].contains("gh pr"));
    assert!(a.windows(2).any(|w| w == ["--output-format", "stream-json"]));
    assert!(a.iter().any(|x| x == "--verbose"), "stream-json needs it with -p");
    assert!(!a.iter().any(|x| x == "--bare"), "it would ignore a subscription's login");
    let b = turn_args("ship", Some("s1"), true);
    assert!(!b.iter().any(|x| x == "--disallowedTools"));
    assert!(b.windows(2).any(|w| w == ["--resume", "s1"]));
}

#[tokio::test]
async fn nothing_ships_without_the_person() {
    let u = up(true);
    u.fake.turns.lock().unwrap().push(vec![
        line(json!({ "type": "system", "subtype": "init", "session_id": "s1" })),
        done("Finished the work."),
    ]);
    let a = u.m.send(&u.ws, "Fix it").await.unwrap();
    let got = settled(&u.m, &a.id).await;
    assert_eq!((got.status, got.ask.clone()), (AgentStatus::NeedsYou, Some(Ask::Ship)));
    assert_eq!(got.session.as_deref(), Some("s1"));
    assert!(!got.may_ship);
    u.m.ship(&a.id).await.unwrap();
    for _ in 0..200 {
        if u.m.state().agents[0].status == AgentStatus::Done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(u.m.state().agents[0].status, AgentStatus::Done);
    let seen = u.fake.seen.lock().unwrap();
    assert!(seen[0].iter().any(|x| x == "--disallowedTools"));
    assert!(!seen[1].iter().any(|x| x == "--disallowedTools"));
    assert!(seen[1].windows(2).any(|w| w == ["--resume", "s1"]));
}

#[tokio::test]
async fn a_busy_wadspace_says_so_and_nonsense_is_refused() {
    let u = up(true);
    let a = u.m.send(&u.ws, "One").await.unwrap();
    settled(&u.m, &a.id).await;
    let e = u.m.send(&u.ws, "Two").await.unwrap_err();
    assert!(e.message.contains("is busy with “One”"), "{}", e.message);
    assert_eq!(u.m.send(&u.ws, "  ").await.unwrap_err().message, "Say what it is first");
}

#[tokio::test]
async fn the_whole_demo_over_a_real_socket() {
    let u = up(true);
    u.m.serve(&u.ws).unwrap();
    let sock = u.sockets.join(&u.ws).join(SOCKET);
    let a = u.m.send(&u.ws, "Pin wadspaces").await.unwrap();
    settled(&u.m, &a.id).await;

    let mut c = tokio::net::UnixStream::connect(&sock).await.unwrap();
    let (r, mut w) = c.split();
    let mut r = BufReader::new(r).lines();
    let mut ask = async |v: Value| {
        w.write_all(format!("{v}\n").as_bytes()).await.unwrap();
        serde_json::from_str::<Value>(&r.next_line().await.unwrap().unwrap()).unwrap()
    };
    let call = |name: &str, args: Value| json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    let init = ask(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })).await;
    assert_eq!(init["result"]["serverInfo"]["name"], "wadspaces");

    let r1 = ask(call("request_sandbox", json!({ "title": "Pins", "why": "Easier to judge by using it" }))).await;
    assert_eq!(r1["result"]["isError"], false, "{r1}");
    assert!(matches!(u.m.state().agents[0].ask, Some(Ask::Sandbox { .. })));
    assert!(u.boxes.log.lock().unwrap().is_empty(), "nothing's made until it's allowed");

    // A checklist before it's allowed has nowhere to go.
    let early = ask(call("set_checklist", json!({ "items": [] }))).await;
    assert_eq!(early["result"]["isError"], true);

    u.m.allow(&a.id).await.unwrap();
    settled(&u.m, &a.id).await;
    let items = json!({ "items": [{ "id": "menu", "label": "It's in the menu", "how": "Right-click", "done": 4 }] });
    assert_eq!(ask(call("set_checklist", items)).await["result"]["isError"], false);
    let bad = ask(call("update_sandbox", json!({ "build": 1, "path": "../../etc" }))).await;
    assert_eq!(bad["result"]["isError"], true, "a path can't leave the home");
    let good = ask(call("update_sandbox", json!({ "build": 1, "path": "target/debug", "run": "./app" }))).await;
    assert_eq!(good["result"]["isError"], false, "{good}");

    let s = u.m.state().sandboxes[0].clone();
    assert_eq!((s.state, s.build, s.items.len()), (SandboxState::Ready, 1, 1));
    u.m.mark(&s.id, "menu", Some(Verdict::Issue), "It isn't").unwrap();
    u.m.send_back(&s.id).unwrap();
    settled(&u.m, &a.id).await;
    let told = u.fake.seen.lock().unwrap().last().unwrap().clone();
    assert!(told[1].contains("• It's in the menu: It isn't"), "{}", told[1]);

    u.m.ship(&a.id).await.unwrap();
    assert_eq!(*u.boxes.log.lock().unwrap(), ["open", "update Some(\"target/debug\")", "close"]);
}

#[tokio::test]
async fn with_the_switch_off_theres_no_socket_and_no_tools() {
    let u = up(false);
    u.m.serve(&u.ws).unwrap();
    assert!(!u.sockets.join(&u.ws).join(SOCKET).exists());
    let e = u.m.tool(&u.ws, "notify", &json!({ "title": "x" })).await.unwrap_err();
    assert!(e.contains("doesn't let agents"), "{e}");
}

#[test]
fn a_build_stays_under_the_home() {
    assert!(in_home("target/debug"));
    for bad in ["", "/etc", "../x", "a/../../b"] {
        assert!(!in_home(bad), "{bad}");
    }
    assert_eq!(shell_word("it's"), r"'it'\''s'");
}
