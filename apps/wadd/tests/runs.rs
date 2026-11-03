//! Running wadspaces against the fake podman and systemd and fake screen
//! hands: opening (its unit, extras, socket, readiness), stopping,
//! reconciling, and the screen's sockets when the compositor restarts.

mod common;

use std::time::Duration;

use common::{drain, read, rig};
use wad_gen::BASE_IMAGE;
use wad_model::{AppId, Phase, SessionDisplay};
use wad_proto::{ErrorCode, Event, ScreenFrom};

fn phases(events: &[Event], id: &str) -> Vec<Phase> {
    let mut out: Vec<Phase> = vec![];
    for e in events {
        if let Event::Run(r) = e
            && r.id == id
            && out.last() != Some(&r.state.phase)
        {
            out.push(r.state.phase);
        }
    }
    out
}

#[tokio::test]
async fn opening_writes_its_unit_and_extras_serves_its_socket_and_waits_for_its_desktop() {
    let rig = rig();
    rig.builds.refresh_base().await;
    rig.fake.state.lock().unwrap().desktop_after = 3;
    let id = rig.wadspace("Plain", &[AppId::Terminal], &[]);
    let (_, mut rx) = rig.bus.subscribe();
    let st = rig.runs.open(&id, SessionDisplay::Window).unwrap();
    assert_eq!((st.state.phase, st.display), (Phase::Starting, Some(SessionDisplay::Window)));
    rig.runs.settled(&id).await;
    let st = rig.runs.state(&id);
    assert_eq!(st.state.phase, Phase::Ready, "{st:?}");
    assert_eq!(phases(&drain(&mut rx), &id), [Phase::Starting, Phase::Waiting, Phase::Ready]);

    // Its unit: the base, its screen folder, its extra folder.
    let unit = rig.fake.state.lock().unwrap().units[&format!("wad-{id}.container")].clone();
    assert!(unit.contains(&format!("Image={BASE_IMAGE}")), "{unit}");
    assert!(
        unit.contains(&format!("Volume={}:/run/wadspaces-display:ro,rslave", rig.screen_dir(&id).display())),
        "{unit}"
    );
    assert!(unit.contains("UIDMap=1000:0:1"), "rootless: abc is you");
    let calls = rig.fake.calls();
    let reload = calls.iter().position(|c| c == "reload").expect("a reload");
    let start = calls.iter().position(|c| *c == format!("start wad-{id}.service")).expect("a start");
    assert!(reload < start);
    let extra = rig.state_dir().join("extra").join(&id);
    assert_eq!(read(&extra.join("display")), "screen\n");
    assert!(read(&extra.join("layout.json")).contains("\"app\": \"terminal\""));
    // A socket of its own, served as io.wadspaces.<id>, in its folder.
    let listens = rig.hands.listens.lock().unwrap().clone();
    assert_eq!(listens.len(), 1);
    assert_eq!(listens[0].0, format!("io.wadspaces.{id}"));
    assert_eq!(listens[0].1, rig.screen_dir(&id).join("wayland-1"));
    assert!(rig.screen.served(&id));
    // The screen loop's tick: its clipboard bridged once its desktop is up.
    rig.screen.tick(rig.runs.on_screen()).await;
    assert!(rig.screen.bridged(&id));
    assert_eq!(rig.hands.listens.lock().unwrap().len(), 1, "served once");

    // Opened again: only shown another way (the shell places it).
    let again = rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    assert_eq!((again.state.phase, again.display), (Phase::Ready, Some(SessionDisplay::Screen)));
    assert_eq!(rig.fake.calls().iter().filter(|c| c.starts_with("start ")).count(), 1);
}

#[tokio::test]
async fn what_isnt_built_or_has_no_screen_doesnt_open() {
    let mut rig = rig();
    rig.builds.refresh_base().await;
    let id = rig.wadspace("Node", &[AppId::Terminal], &["terminal:nodejs"]);
    let e = rig.runs.open(&id, SessionDisplay::Screen).unwrap_err();
    assert_eq!(e.code, ErrorCode::Conflict);
    assert!(e.message.contains("build it first"), "{}", e.message);
    assert_eq!(rig.runs.open("nope", SessionDisplay::Screen).unwrap_err().code, ErrorCode::NotFound);
    let plain = rig.wadspace("Plain", &[AppId::Terminal], &[]);
    assert_eq!(rig.runs.open(&plain, SessionDisplay::Stream).unwrap_err().code, ErrorCode::BadRequest);
    // No compositor anywhere.
    rig.listener = None;
    std::fs::remove_file(&rig.compositor).unwrap();
    assert_eq!(rig.runs.open(&plain, SessionDisplay::Screen).unwrap_err().code, ErrorCode::Offline);
    assert_eq!(rig.screen.info().from, ScreenFrom::None);
}

#[tokio::test]
async fn stopping_takes_its_socket_and_stops_its_unit() {
    let rig = rig();
    rig.builds.refresh_base().await;
    let id = rig.wadspace("Plain", &[AppId::Terminal], &[]);
    rig.runs.open(&id, SessionDisplay::Background).unwrap();
    rig.runs.settled(&id).await;
    let st = rig.runs.stop(&id).await.unwrap();
    assert_eq!((st.state.phase, st.display), (Phase::Idle, None));
    assert!(rig.fake.calls().contains(&format!("stop wad-{id}.service")));
    assert!(!rig.screen.served(&id));
    assert_eq!(rig.hands.removed.lock().unwrap().clone(), [rig.screen_dir(&id).join("wayland-1")]);
    // Stopping what's stopped is nothing.
    let n = rig.fake.calls().len();
    assert_eq!(rig.runs.stop(&id).await.unwrap().state.phase, Phase::Idle);
    assert_eq!(rig.fake.calls().len(), n);
}

#[tokio::test]
async fn reconcile_follows_containers_that_stop_by_themselves_or_ran_before() {
    let rig = rig();
    rig.builds.refresh_base().await;
    let a = rig.wadspace("A", &[AppId::Terminal], &[]);
    rig.runs.open(&a, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&a).await;
    // Its window was closed: its desktop halts the container.
    rig.fake.exit(&format!("wad-{a}"));
    rig.runs.reconcile().await;
    assert_eq!(rig.runs.state(&a).state.phase, Phase::Idle);
    rig.screen.tick(rig.runs.on_screen()).await;
    assert!(!rig.screen.served(&a));

    // One running before wadd started: taken on, and served.
    let b = rig.wadspace("B", &[AppId::Terminal], &[]);
    rig.fake.run_container(&format!("wad-{b}"), BASE_IMAGE);
    rig.runs.reconcile().await;
    let st = rig.runs.state(&b);
    assert_eq!((st.state.phase, st.display), (Phase::Ready, Some(SessionDisplay::Screen)));
    rig.screen.tick(rig.runs.on_screen()).await;
    assert!(rig.screen.served(&b));

    // Podman not answering changes nothing.
    rig.fake.state.lock().unwrap().down = true;
    rig.runs.reconcile().await;
    assert_eq!(rig.runs.state(&b).state.phase, Phase::Ready);
}

#[tokio::test]
async fn a_restarted_compositor_serves_its_sockets_again() {
    let mut rig = rig();
    rig.builds.refresh_base().await;
    let id = rig.wadspace("Plain", &[AppId::Terminal], &[]);
    rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&id).await;
    rig.screen.tick(rig.runs.on_screen()).await;
    let first = rig.hands.listens.lock().unwrap()[0].2;
    // The same compositor: nothing made again.
    rig.screen.tick(rig.runs.on_screen()).await;
    assert_eq!(rig.hands.listens.lock().unwrap().len(), 1);

    rig.restart_compositor();
    rig.screen.tick(rig.runs.on_screen()).await;
    let listens = rig.hands.listens.lock().unwrap().clone();
    assert_eq!(listens.len(), 2);
    assert_ne!(listens[1].2, first, "the new compositor's");
    assert_eq!(listens[1].1, listens[0].1, "at the same path");

    // Gone: its sockets are taken away, and made again when it's back.
    rig.listener = None;
    std::fs::remove_file(&rig.compositor).unwrap();
    rig.screen.tick(rig.runs.on_screen()).await;
    assert!(!rig.screen.served(&id));
    rig.listener = Some(std::os::unix::net::UnixListener::bind(&rig.compositor).unwrap());
    rig.screen.tick(rig.runs.on_screen()).await;
    assert!(rig.screen.served(&id));
    assert_eq!(rig.hands.listens.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn the_shells_session_is_the_screen_until_its_socket_goes() {
    let rig = rig();
    let me = nix::unistd::getuid().as_raw();
    // The runtime dir's newest wayland-*, until the shell says.
    assert_eq!(rig.screen.info().from, ScreenFrom::RuntimeDir);
    let shell = rig.dir.path().join("shell-wayland-1");
    let held = std::os::unix::net::UnixListener::bind(&shell).unwrap();
    let info = rig.screen.register(&shell.to_string_lossy(), me, me).unwrap();
    assert_eq!((info.from, info.socket.as_deref()), (ScreenFrom::Shell, Some(&*shell.to_string_lossy())));
    // Only your own, only a socket, only an absolute path.
    assert_eq!(rig.screen.register(&shell.to_string_lossy(), me + 1, me).unwrap_err().code, ErrorCode::Forbidden);
    assert_eq!(rig.screen.register("wayland-0", me, me).unwrap_err().code, ErrorCode::BadRequest);
    let file = rig.dir.path().join("not-a-socket");
    std::fs::write(&file, "").unwrap();
    assert_eq!(rig.screen.register(&file.to_string_lossy(), me, me).unwrap_err().code, ErrorCode::BadRequest);
    drop(held);
    std::fs::remove_file(&shell).unwrap();
    assert_eq!(rig.screen.info().from, ScreenFrom::RuntimeDir);
}

#[tokio::test]
async fn a_desktop_that_never_comes_up_is_an_error_and_one_that_stops_says_so() {
    let rig = rig();
    rig.builds.refresh_base().await;
    rig.fake.state.lock().unwrap().desktop_after = u32::MAX;
    let id = rig.wadspace("Slow", &[AppId::Terminal], &[]);
    rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    // Its container stops while it waits.
    tokio::time::sleep(Duration::from_millis(50)).await;
    rig.fake.exit(&format!("wad-{id}"));
    rig.runs.settled(&id).await;
    let st = rig.runs.state(&id);
    assert_eq!(st.state.phase, Phase::Error);
    assert!(st.state.error.as_deref().unwrap().contains("stopped before its desktop was up"), "{st:?}");
    // It can be opened again.
    rig.fake.state.lock().unwrap().desktop_after = 0;
    rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&id).await;
    assert_eq!(rig.runs.state(&id).state.phase, Phase::Ready);
}

#[tokio::test]
async fn a_wadspace_trashed_stops_and_one_deleted_for_good_is_forgotten() {
    let rig = rig();
    rig.builds.refresh_base().await;
    let id = rig.wadspace("Trash me", &[AppId::Terminal], &[]);
    rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&id).await;
    let out = rig.docs.apply(wad_model::Op::Trash { ws: id.clone() }).unwrap();
    rig.runs.edited(&out.changed).await;
    assert_eq!(rig.runs.state(&id).state.phase, Phase::Idle);
    assert!(rig.fake.calls().contains(&format!("stop wad-{id}.service")));

    // An empty one discarded: gone for good, and what wadd kept for it too.
    let empty = rig.wadspace("Empty", &[], &[]);
    rig.runs.open(&empty, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&empty).await;
    assert!(rig.fake.state.lock().unwrap().units.contains_key(&format!("wad-{empty}.container")));
    let out = rig.docs.apply(wad_model::Op::DiscardIfEmpty { ws: empty.clone() }).unwrap();
    assert_eq!(out.changed, std::slice::from_ref(&empty));
    rig.runs.edited(&out.changed).await;
    assert!(rig.fake.calls().contains(&format!("stop wad-{empty}.service")));
    assert!(!rig.fake.state.lock().unwrap().units.contains_key(&format!("wad-{empty}.container")));
    assert!(!rig.state_dir().join("extra").join(&empty).exists());
    assert!(rig.runs.states().iter().all(|r| r.id != empty));
}

#[tokio::test]
async fn a_reconcile_while_its_starting_leaves_it_starting_as_it_was_shown() {
    // (The real race is across threads: the Starting event wakes reconcile
    // before open() has registered the bring-up. open() now does both under
    // the task map's lock, and reconcile never takes a Starting one for
    // stopped; this checks what follows from that.)
    let rig = rig();
    rig.builds.refresh_base().await;
    let id = rig.wadspace("Racy", &[AppId::Terminal], &[]);
    // Opened, and reconciled before its bring-up has made its container.
    rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    rig.runs.reconcile().await;
    let st = rig.runs.state(&id);
    assert_ne!(st.state.phase, Phase::Idle, "taken for stopped: {st:?}");
    assert_eq!(st.display, Some(SessionDisplay::Screen));
    rig.runs.settled(&id).await;
    let st = rig.runs.state(&id);
    assert_eq!((st.state.phase, st.display), (Phase::Ready, Some(SessionDisplay::Screen)), "{st:?}");
}

#[tokio::test]
async fn one_of_wadspaces_own_pictures_comes_with_it() {
    let rig = rig();
    rig.builds.refresh_base().await;
    let id = rig.wadspace("Dunes", &[AppId::Terminal], &[]);
    let look = |value: &str| wad_model::edit::Op::SetWallpaper {
        ws: id.clone(),
        wallpaper: wad_model::Wallpaper::Image { value: value.into() },
    };
    rig.docs.apply(look("/wallpapers/art/dunes.svg")).unwrap();
    rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&id).await;
    let copied = rig.state_dir().join("extra").join(&id).join("wallpaper.svg");
    assert!(read(&copied).contains("<svg"), "the art's picture is in its extra folder");
    // A path that climbs out of the pictures isn't taken.
    rig.runs.stop(&id).await.unwrap();
    rig.docs.apply(look("/wallpapers/../../../etc/hostname")).ok();
    rig.runs.open(&id, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&id).await;
    assert!(!rig.state_dir().join("extra").join(&id).join("wallpaper.svg").exists());
}
