//! Real sockets on 127.0.0.1: reliable messages and a short lockstep session.

use marionette_net::{Client, Delivery, Endpoint, Host, LockstepConfig, Transport, UdpDatagram};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn bind() -> Endpoint<UdpDatagram> {
    Endpoint::new(UdpDatagram::bind("127.0.0.1:0".parse().unwrap()).unwrap())
}

#[test]
fn reliable_over_udp() {
    let mut a = bind();
    let mut b = bind();
    let big = vec![7u8; 200_000];
    a.send(b.local(), Delivery::Reliable, 2, &big).unwrap();
    a.send(b.local(), Delivery::Unreliable, 1, b"hello").unwrap();
    let start = Instant::now();
    let mut got = Vec::new();
    while got.len() < 2 && start.elapsed() < Duration::from_secs(10) {
        let now = start.elapsed().as_micros() as u64;
        a.poll(now);
        b.poll(now);
        while let Some(p) = b.receive() {
            got.push(p);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(got.len(), 2, "both arrive");
    let rel = got.iter().find(|p| p.delivery == Delivery::Reliable).unwrap();
    assert_eq!(rel.data, big);
    assert_eq!(rel.from, a.local());
}

#[test]
fn lockstep_over_udp() {
    let cfg = LockstepConfig {
        actions: vec!["fire".into()],
        ..LockstepConfig::default()
    };
    let mut ht = bind();
    let mut ct = bind();
    let mut host = Host::new(cfg.clone(), ht.local(), 0, &[ct.local()]);
    let mut client = Client::new(cfg, ht.local());
    let start = Instant::now();
    let mut run = Vec::new();
    let mut ran = Vec::new();
    while ran.len() < 120 && start.elapsed() < Duration::from_secs(20) {
        let now = start.elapsed().as_micros() as u64;
        host.poll(&mut ht, now);
        if host.next_tick() < 120 {
            let v = if host.next_tick() % 10 < 5 { 1.0 } else { 0.0 };
            host.set_local(&BTreeMap::from([("fire".to_string(), v)]));
            run.push(host.next_frame());
        }
        host.flush(&mut ht);
        client.poll(&mut ct, now);
        while let Some(f) = client.next_frame() {
            ran.push(f);
        }
        client.flush(&mut ct);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(ran, run, "the client runs exactly the host's frames");
    assert_eq!(client.slot(), Some(1));
}
