//! wad-bt against the fake BlueZ on a private D-Bus: the adapter and its
//! devices, power, looking for devices, and pairing through the agent
//! (a code to confirm, a PIN to enter, turned down).

use tokio::sync::mpsc;
use wad_bt::fake::{Asks, FakeBluez, FakeDevice, W, World};
use wad_bt::{AgentEvent, Answer, Bluez, Error};
use wad_net::fake::PrivateBus;
use wad_proto::{BtAsk, BtKind};

const BUDS: &str = "AA:BB:CC:00:00:01";
const MOUSE: &str = "AA:BB:CC:00:00:02";
const SPEAKER: &str = "AA:BB:CC:00:00:03";
const OLD: &str = "AA:BB:CC:00:00:04";
const BEACON: &str = "AA:BB:CC:00:00:05";

struct Up {
    _bus: PrivateBus,
    _bz: FakeBluez,
    world: W,
    bt: Bluez,
    events: mpsc::UnboundedReceiver<AgentEvent>,
}

async fn up() -> Up {
    let bus = PrivateBus::start().await;
    let mut beacon = FakeDevice::new(BEACON, "", "");
    beacon.name = None;
    let world = World::with(
        "wados",
        vec![
            FakeDevice::new(BUDS, "Pixel Buds Pro", "audio-headphones").paired(true).battery(80),
            FakeDevice::new(MOUSE, "MX Master 3S", "input-mouse").paired(false),
            FakeDevice::new(SPEAKER, "JBL Flip 6", "audio-card").asks(Asks::Confirm(492817)),
            FakeDevice::new(OLD, "Old Keyboard", "input-keyboard").asks(Asks::Pin("0000")),
            beacon,
        ],
    );
    let bz = FakeBluez::serve(&bus.address, world.clone()).await;
    let (tx, events) = mpsc::unbounded_channel();
    let bt = Bluez::on(bus.connect().await, tx).await.unwrap();
    Up { _bus: bus, _bz: bz, world, bt, events }
}

#[tokio::test]
async fn the_adapter_and_its_devices() {
    let b = up().await;
    assert!(b.world.lock().unwrap().agent.is_some() && b.world.lock().unwrap().default_agent, "its agent's registered");
    let s = b.bt.state().await.unwrap().unwrap();
    assert!(s.on && !s.discovering);
    assert_eq!(s.alias, "wados");
    let names: Vec<&str> = s.devices.iter().map(|d| d.name.as_str()).collect();
    // Paired first (connected first); the nameless beacon left out.
    assert_eq!(names, ["Pixel Buds Pro", "MX Master 3S", "JBL Flip 6", "Old Keyboard"]);
    let buds = &s.devices[0];
    assert_eq!((buds.kind, buds.connected, buds.battery), (BtKind::Headphones, true, Some(80)));
    assert_eq!(s.devices[2].kind, BtKind::Speaker);

    b.bt.scan(true).await.unwrap();
    assert!(b.world.lock().unwrap().discoverable, "seen while it looks");
    assert!(b.bt.state().await.unwrap().unwrap().discovering);
    b.bt.scan(false).await.unwrap();
    b.bt.scan(false).await.unwrap(); // stopped already: fine

    b.bt.power(false).await.unwrap();
    let s = b.bt.state().await.unwrap().unwrap();
    assert!(!s.on && s.devices.is_empty());
    assert!(matches!(b.bt.connect(MOUSE).await.unwrap_err(), Error::Off));
    assert!(matches!(b.bt.scan(true).await.unwrap_err(), Error::Off));
    b.bt.power(true).await.unwrap();
    b.bt.connect(MOUSE).await.unwrap();
    b.bt.disconnect(BUDS).await.unwrap();
    let s = b.bt.state().await.unwrap().unwrap();
    assert_eq!(s.devices[0].name, "MX Master 3S", "connected first");
    assert!(matches!(b.bt.connect("AA:BB:CC:99:99:99").await.unwrap_err(), Error::NotFound(_)));
    assert!(matches!(b.bt.connect("../x").await.unwrap_err(), Error::BadAddress(_)));
    assert!(matches!(b.bt.connect(SPEAKER).await.unwrap_err(), Error::Refused(_)), "not paired");
    b.bt.forget(MOUSE).await.unwrap();
    assert!(!b.bt.state().await.unwrap().unwrap().devices.iter().any(|d| d.address == MOUSE && d.paired));
}

/// The next thing the agent asks.
async fn asked(events: &mut mpsc::UnboundedReceiver<AgentEvent>) -> wad_bt::Ask {
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(10), events.recv()).await {
            Ok(Some(AgentEvent::Ask(a))) => return a,
            Ok(Some(AgentEvent::Done)) => continue,
            other => panic!("nothing asked: {other:?}"),
        }
    }
}

#[tokio::test]
async fn pairing_asks_through_the_agent() {
    let mut b = up().await;
    let bt = std::sync::Arc::new(b.bt);
    // A code to confirm: yes pairs, trusts and connects it.
    let pairing = tokio::spawn({
        let bt = bt.clone();
        async move { bt.pair(SPEAKER).await }
    });
    let a = asked(&mut b.events).await;
    assert_eq!((a.prompt.device.as_str(), a.prompt.name.as_str()), (SPEAKER, "JBL Flip 6"));
    assert_eq!(a.prompt.kind, BtAsk::Confirm { code: "492817".into() });
    assert!(bt.state().await.unwrap().unwrap().busy.contains(&SPEAKER.to_string()));
    a.reply.unwrap().send(Answer { accept: true, pin: None }).unwrap();
    pairing.await.unwrap().unwrap();
    {
        let w = b.world.lock().unwrap();
        let d = w.devices.iter().find(|d| d.address == SPEAKER).unwrap();
        assert!(d.paired && d.trusted && d.connected);
    }

    // A PIN: the right one pairs it.
    let pairing = tokio::spawn({
        let bt = bt.clone();
        async move { bt.pair(OLD).await }
    });
    let a = asked(&mut b.events).await;
    assert_eq!(a.prompt.kind, BtAsk::Enter);
    a.reply.unwrap().send(Answer { accept: true, pin: Some("0000".into()) }).unwrap();
    pairing.await.unwrap().unwrap();

    // Turned down: it isn't paired, and it says why.
    b.world.lock().unwrap().devices.iter_mut().find(|d| d.address == SPEAKER).unwrap().paired = false;
    let pairing = tokio::spawn({
        let bt = bt.clone();
        async move { bt.pair(SPEAKER).await }
    });
    let a = asked(&mut b.events).await;
    a.reply.unwrap().send(Answer { accept: false, pin: None }).unwrap();
    let e = pairing.await.unwrap().unwrap_err();
    assert!(e.to_string().starts_with("Pairing was turned down"), "{e}");
    assert!(!b.world.lock().unwrap().devices.iter().find(|d| d.address == SPEAKER).unwrap().paired);
}
