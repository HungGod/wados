//! Focus on the fake machine: what it holds, and how it ends.

mod common;

use std::sync::Arc;

use common::{Rig, drain, rig};
use wad_model::{AppId, Phase, SessionDisplay};
use wad_proto::Event;
use wadd::focus::MachineFocus;

const MINUTE: i64 = 60_000;

fn focus(rig: &Rig) -> Arc<MachineFocus> {
    MachineFocus::new(rig.dir.path(), rig.docs.clone(), rig.runs.clone(), rig.bus.clone())
}

async fn two(rig: &Rig) -> (String, String) {
    rig.builds.refresh_base().await;
    (rig.wadspace("Work", &[AppId::Terminal], &[]), rig.wadspace("Play", &[AppId::Terminal], &[]))
}

#[tokio::test]
async fn it_opens_full_screen_and_sends_the_others_to_the_background() {
    let rig = rig();
    let (work, play) = two(&rig).await;
    rig.runs.open(&play, SessionDisplay::Screen).unwrap();
    rig.runs.settled(&play).await;
    let f = focus(&rig);
    let on = f.start(&work, 25).unwrap();
    assert_eq!(on.until - on.started, 25 * MINUTE);
    assert_eq!(rig.runs.state(&work).display, Some(SessionDisplay::Screen));
    assert_eq!(rig.runs.state(&play).display, Some(SessionDisplay::Background));
}

#[tokio::test]
async fn nothing_leaves_it_or_takes_its_place_until_the_times_up() {
    let rig = rig();
    let (work, play) = two(&rig).await;
    let f = focus(&rig);
    f.start(&work, 25).unwrap();
    let held = |e: wad_proto::ApiError| assert!(e.message.starts_with("Focusing in Work"), "{}", e.message);
    held(f.check_stop(&work).unwrap_err());
    held(f.check_stream(&work).unwrap_err());
    held(f.check_free().unwrap_err());
    held(f.start(&play, 5).unwrap_err());
    for d in [SessionDisplay::Window, SessionDisplay::Background, SessionDisplay::Stream] {
        held(f.check_open(&work, d).unwrap_err());
    }
    for d in [SessionDisplay::Screen, SessionDisplay::Window, SessionDisplay::Background] {
        held(f.check_open(&play, d).unwrap_err());
    }
    // Being on the screen is what it's for, and another's stream isn't on this screen.
    f.check_open(&work, SessionDisplay::Screen).unwrap();
    f.check_open(&play, SessionDisplay::Stream).unwrap();
    f.check_stop(&play).unwrap();
    f.check_stream(&play).unwrap();
}

#[tokio::test]
async fn the_time_it_can_be_set_for_has_limits() {
    let rig = rig();
    let (work, _) = two(&rig).await;
    let f = focus(&rig);
    assert_eq!(f.start(&work, 0).unwrap_err().message, "Set at least a minute");
    assert_eq!(f.start(&work, 12 * 60 + 1).unwrap_err().message, "At most 12 hours");
    assert!(f.start("nothing", 5).is_err());
    assert_eq!(f.get(), None);
}

fn notices(events: &[Event]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Notice { title, body, .. } => Some((title.clone(), body.clone().unwrap_or_default())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn time_up_says_so_and_the_wadspace_stays() {
    let rig = rig();
    let (work, _) = two(&rig).await;
    let f = focus(&rig);
    let on = f.start(&work, 25).unwrap();
    rig.runs.settled(&work).await;
    let (_, mut rx) = rig.bus.subscribe();
    f.tick(on.until - 1);
    assert!(f.get().is_some());
    f.tick(on.until);
    assert_eq!(f.get(), None);
    let n = notices(&drain(&mut rx));
    assert_eq!(n, [("Focus is over".to_string(), "25 minutes in Work. You can leave it now.".to_string())]);
    assert_eq!(rig.runs.state(&work).state.phase, Phase::Ready);
    f.check_stop(&work).unwrap();
}

#[tokio::test]
async fn a_wadspace_that_doesnt_open_ends_it() {
    let rig = rig();
    let (work, _) = two(&rig).await;
    let f = focus(&rig);
    let on = f.start(&work, 25).unwrap();
    rig.runs.settled(&work).await;
    f.tick(on.started + 1000);
    rig.runs.stop(&work).await.unwrap();
    let (_, mut rx) = rig.bus.subscribe();
    f.tick(on.started + 2000);
    assert_eq!(f.get(), None);
    assert_eq!(notices(&drain(&mut rx))[0].0, "Focus is over");
}

#[tokio::test]
async fn wadd_restarting_isnt_a_way_out_but_a_finished_one_isnt_kept() {
    let rig = rig();
    let (work, _) = two(&rig).await;
    let on = focus(&rig).start(&work, 25).unwrap();
    let again = focus(&rig);
    assert_eq!(again.get(), Some(on.clone()));
    assert!(again.check_stop(&work).is_err());
    again.tick(on.until);
    assert_eq!(focus(&rig).get(), None);
}

mod server {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use wadd::server::{Hands, ServerMode};

    use super::*;

    #[derive(Default)]
    struct Seat(Mutex<Vec<&'static str>>);

    #[async_trait]
    impl Hands for Seat {
        async fn seat_off(&self) -> Result<(), String> {
            self.0.lock().unwrap().push("off");
            Ok(())
        }
        async fn seat_on(&self) -> Result<(), String> {
            self.0.lock().unwrap().push("on");
            Ok(())
        }
    }

    fn mode(rig: &Rig, f: &Arc<MachineFocus>, seat: &Arc<Seat>) -> Arc<ServerMode> {
        ServerMode::new(rig.dir.path(), rig.runs.clone(), f.clone(), seat.clone(), rig.bus.clone())
    }

    #[tokio::test]
    async fn the_screen_goes_and_streams_stay_and_it_outlasts_wadd() {
        let rig = rig();
        let (work, _) = two(&rig).await;
        rig.runs.open(&work, SessionDisplay::Screen).unwrap();
        rig.runs.settled(&work).await;
        let (f, seat) = (focus(&rig), Arc::new(Seat::default()));
        let s = mode(&rig, &f, &seat);
        assert!(s.set(true).await.unwrap().on);
        assert_eq!(rig.runs.state(&work).state.phase, Phase::Idle);
        assert!(s.check_open(SessionDisplay::Screen).is_err());
        assert!(s.check_open(SessionDisplay::Window).is_err());
        s.check_open(SessionDisplay::Stream).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert_eq!(*seat.0.lock().unwrap(), ["off"]);

        // wadd restarting doesn't bring the screen back.
        let again = mode(&rig, &f, &seat);
        assert!(again.get().on);
        // The caller going away (the console it stops) doesn't stop it coming back.
        let leaving = again.clone();
        let call = tokio::spawn(async move { leaving.set(false).await });
        let _ = call.await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!again.get().on);
        assert_eq!(*seat.0.lock().unwrap(), ["off", "on"]);
        assert!(!mode(&rig, &f, &seat).get().on);
    }

    #[tokio::test]
    async fn focus_holds_it_off() {
        let rig = rig();
        let (work, _) = two(&rig).await;
        let (f, seat) = (focus(&rig), Arc::new(Seat::default()));
        f.start(&work, 5).unwrap();
        let s = mode(&rig, &f, &seat);
        assert!(s.set(true).await.unwrap_err().message.starts_with("Focusing in Work"));
        assert!(!s.get().on);
    }
}
