//! wad-net against the fake NetworkManager on a private D-Bus: the
//! networks in range and saved, joining with the password over D-Bus, a
//! wrong one leaving no profile, forgetting, the radio, and the wire.

use wad_net::fake::{FakeAp, FakeNm, PrivateBus, W, World, get_str, saved_profile, ssid_of};
use wad_net::{Error, NetworkManager};
use wad_proto::WifiBand;

struct Up {
    _bus: PrivateBus,
    _nm: FakeNm,
    world: W,
    nm: NetworkManager,
}

async fn up(world: W) -> Up {
    let bus = PrivateBus::start().await;
    let fake = FakeNm::serve(&bus, world.clone()).await;
    let nm = NetworkManager::on(bus.connect().await);
    Up { _bus: bus, _nm: fake, world, nm }
}

fn home() -> Vec<FakeAp> {
    vec![
        FakeAp::secure("Home", 40),
        FakeAp::secure("Home", 70),
        FakeAp::open("Cafe", 90),
        FakeAp::enterprise("Uni", 60),
        FakeAp::secure("", 80),
    ]
}

#[tokio::test]
async fn the_networks_in_range_and_saved() {
    let world = World::with(home(), &[("Office", "x")]);
    saved_profile(&world, "Office", true);
    let b = up(world).await;
    let s = b.nm.state().await.unwrap();
    assert!(s.has_wifi && s.wifi_on && s.wifi_hardware);
    let names: Vec<&str> = s.networks.iter().map(|n| n.ssid.as_str()).collect();
    // The saved one first (out of range: no signal), then by signal; one Home; no hidden one.
    assert_eq!(names, ["Office", "Cafe", "Home", "Uni"]);
    let office = &s.networks[0];
    assert_eq!((office.signal, office.saved, office.secure, office.band), (0, true, true, None));
    let home = &s.networks[2];
    assert_eq!((home.signal, home.secure, home.saved, home.band), (70, true, false, Some(WifiBand::Ghz5)));
    assert!(!s.networks[1].secure);
    assert_eq!(home.security.as_deref(), Some("WPA2 Personal"));
    assert_eq!(s.networks[1].security.as_deref(), Some("None (open network)"));
    assert_eq!(s.networks[3].security.as_deref(), Some("WPA2 Enterprise"));
    assert_eq!((s.connected, s.ip, s.ethernet.plugged), (None, None, false));
}

#[tokio::test]
async fn joining_with_the_password_over_dbus() {
    let b = up(World::with(home(), &[("Home", "correct horse")])).await;
    b.nm.connect("Home", Some("correct horse")).await.unwrap();
    {
        let w = b.world.lock().unwrap();
        let sent = &w.added[0];
        assert_eq!(get_str(sent, "802-11-wireless-security", "psk").as_deref(), Some("correct horse"));
        assert_eq!(get_str(sent, "802-11-wireless-security", "key-mgmt").as_deref(), Some("wpa-psk"));
        assert_eq!(ssid_of(sent), "Home");
    }
    let s = b.nm.state().await.unwrap();
    assert_eq!(s.connected.as_deref(), Some("Home"));
    assert!(s.networks[0].active && s.networks[0].saved && s.networks[0].ssid == "Home"); // joined first
    assert_eq!((s.ip.as_deref(), s.dns.clone()), (Some("192.168.1.42"), vec!["192.168.1.1".to_string()]));
    // Later: the saved one, without its password again.
    b.nm.disconnect().await.unwrap();
    assert_eq!(b.nm.state().await.unwrap().connected, None);
    b.nm.connect("Home", None).await.unwrap();
    assert_eq!(b.nm.state().await.unwrap().connected.as_deref(), Some("Home"));
    assert_eq!(b.world.lock().unwrap().profiles.len(), 1);
}

#[tokio::test]
async fn a_wrong_password_says_so_and_leaves_no_profile() {
    let b = up(World::with(home(), &[("Home", "correct horse")])).await;
    let e = b.nm.connect("Home", Some("wrong pass")).await.unwrap_err();
    assert!(matches!(&e, Error::WrongPassword(s) if s == "Home"), "{e}");
    assert_eq!(e.to_string(), "That isn't the password for Home");
    assert!(b.world.lock().unwrap().profiles.is_empty());
    assert_eq!(b.nm.state().await.unwrap().connected, None);
}

#[tokio::test]
async fn what_cant_be_joined_says_why() {
    let b = up(World::with(home(), &[("Home", "correct horse")])).await;
    let msg = |e: Error| e.to_string();
    assert_eq!(
        msg(b.nm.connect("Uni", Some("whatever1")).await.unwrap_err()),
        "Uni is a WPA2 802.1X network: those can't be joined here"
    );
    assert!(matches!(b.nm.connect("Home", None).await.unwrap_err(), Error::NeedsPassword(_)));
    assert!(matches!(b.nm.connect("Home", Some("short")).await.unwrap_err(), Error::BadPassword));
    assert!(matches!(b.nm.connect("Elsewhere", None).await.unwrap_err(), Error::NotInRange(_)));
    assert!(b.world.lock().unwrap().last_scan > 0, "it looked again before saying so");
    assert!(b.world.lock().unwrap().added.is_empty()); // nothing reached NetworkManager
    // An open one needs nothing.
    b.nm.connect("Cafe", None).await.unwrap();
    assert!(!b.world.lock().unwrap().added[0].contains_key("802-11-wireless-security"));
}

#[tokio::test]
async fn forgetting_and_a_new_password() {
    let b = up(World::with(home(), &[("Home", "correct horse")])).await;
    b.nm.connect("Home", Some("correct horse")).await.unwrap();
    // A new password replaces the old profile.
    b.nm.connect("Home", Some("correct horse")).await.unwrap();
    assert_eq!(b.world.lock().unwrap().profiles.len(), 1);
    b.nm.forget("Home").await.unwrap();
    assert!(b.world.lock().unwrap().profiles.is_empty());
    let s = b.nm.state().await.unwrap();
    assert!(s.connected.is_none(), "forgetting the one it's on leaves it");
    assert!(matches!(b.nm.forget("Home").await.unwrap_err(), Error::NotSaved(_)));
}

#[tokio::test]
async fn the_radio_and_the_wire() {
    let world = World::with(home(), &[]);
    world.lock().unwrap().wired = Some((false, 1000));
    let b = up(world).await;
    b.nm.connect("Cafe", None).await.unwrap();
    b.nm.set_wifi(false).await.unwrap();
    let s = b.nm.state().await.unwrap();
    assert!(!s.wifi_on && s.connected.is_none() && s.networks.iter().all(|n| n.saved));
    assert!(matches!(b.nm.connect("Cafe", None).await.unwrap_err(), Error::WifiOff));
    // A cable in: the wire's address is the machine's.
    b.world.lock().unwrap().wired = Some((true, 1000));
    let s = b.nm.state().await.unwrap();
    assert_eq!(
        (s.ethernet.plugged, s.ethernet.speed_mbps, s.ethernet.ip.as_deref(), s.ip.as_deref()),
        (true, Some(1000), Some("192.168.1.24"), Some("192.168.1.24"))
    );
    // Its switch off: it stays off.
    b.world.lock().unwrap().hardware = false;
    assert!(matches!(b.nm.set_wifi(true).await.unwrap_err(), Error::Blocked));
    b.world.lock().unwrap().hardware = true;
    b.nm.set_wifi(true).await.unwrap();
    assert!(b.nm.state().await.unwrap().wifi_on);
}

#[tokio::test]
async fn a_network_that_wants_a_sign_in_says_so() {
    let b = up(World::with(home(), &[])).await;
    b.nm.connect("Cafe", None).await.unwrap();
    assert!(!b.nm.state().await.unwrap().portal, "the check found the internet");
    // The café's page in the way (NetworkManager's connectivity check).
    b.world.lock().unwrap().connectivity = wad_net::PORTAL;
    let s = b.nm.state().await.unwrap();
    assert!(s.portal && s.connected.as_deref() == Some("Cafe"));
}
