use rx_melsec_mc::{
    AccessProfile, AddressRange, Client, Configuration, Environment, Error, FailureStage, Route,
};
use std::{
    io::{Read, Write},
    net::{SocketAddrV4, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn configuration(endpoint: SocketAddrV4) -> Configuration {
    Configuration {
        environment: Environment::Simulation,
        endpoint,
        route: Route {
            network: 0,
            pc: 255,
            module_io: 0x03ff,
            station: 0,
        },
        monitoring_timer: 16,
        connect_timeout_ms: 500,
        exchange_timeout_ms: 500,
        access: AccessProfile {
            m_last: 999,
            d_last: 999,
            read_m: vec![AddressRange {
                first: 100,
                last: 199,
            }],
            read_d: vec![AddressRange {
                first: 350,
                last: 399,
            }],
            write_m: [200].into(),
        },
    }
}
fn server(f: impl FnOnce(TcpStream) + Send + 'static) -> (Configuration, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let std::net::SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
        panic!()
    };
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        f(stream);
    });
    (configuration(endpoint), handle)
}
fn request(stream: &mut TcpStream) -> Vec<u8> {
    let mut header = [0; 9];
    stream.read_exact(&mut header).unwrap();
    let len = usize::from(u16::from_le_bytes([header[7], header[8]]));
    assert!((12..=13).contains(&len));
    let mut body = vec![0; len];
    stream.read_exact(&mut body).unwrap();
    [header.to_vec(), body].concat()
}
fn response(payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0xd0, 0, 0, 255, 255, 3, 0];
    frame.extend(u16::try_from(payload.len() + 2).unwrap().to_le_bytes());
    frame.extend([0, 0]);
    frame.extend(payload);
    frame
}

#[test]
fn documented_bit_read_and_word_read_vectors_over_fragmented_tcp() {
    let (config, handle) = server(|mut stream| {
        // SH-080008AB printed p91: M100..M107, binary bit-unit read.
        assert_eq!(
            request(&mut stream),
            [
                0x50, 0, 0, 255, 255, 3, 0, 12, 0, 16, 0, 1, 4, 1, 0, 100, 0, 0, 0x90, 8, 0
            ]
        );
        for b in response(&[0, 1, 0, 0x11]) {
            stream.write_all(&[b]).unwrap();
        }
        // D350/D351 lower-byte-first data from printed p74.
        assert_eq!(
            request(&mut stream),
            [
                0x50, 0, 0, 255, 255, 3, 0, 12, 0, 16, 0, 1, 4, 0, 0, 0x5e, 1, 0, 0xa8, 2, 0
            ]
        );
        stream
            .write_all(&response(&[0xab, 0x56, 0x0f, 0x17]))
            .unwrap();
    });
    let mut client = Client::connect(config).unwrap();
    assert_eq!(
        client.read_m(100, 8).unwrap(),
        [false, false, false, true, false, false, true, true]
    );
    assert_eq!(client.read_d(350, 2).unwrap(), [0x56ab, 0x170f]);
    assert!(!client.is_faulted());
    handle.join().unwrap();
}

#[test]
fn mapped_single_bit_write_ack_does_not_claim_physical_completion() {
    let (config, handle) = server(|mut stream| {
        assert_eq!(
            request(&mut stream),
            [
                0x50, 0, 0, 255, 255, 3, 0, 13, 0, 16, 0, 1, 0x14, 1, 0, 200, 0, 0, 0x90, 1, 0,
                0x10
            ]
        );
        stream.write_all(&response(&[])).unwrap();
        assert_eq!(request(&mut stream).last(), Some(&0));
        stream.write_all(&response(&[])).unwrap();
    });
    let mut client = Client::connect(config).unwrap();
    let ack = client.write_m(200, true).unwrap();
    assert_eq!(ack.address, 200);
    assert!(ack.requested_value);
    assert!(!client.write_m(200, false).unwrap().requested_value);
    handle.join().unwrap();
}

#[test]
fn lost_write_response_preserves_uncertainty_and_never_resends() {
    let effects = Arc::new(AtomicUsize::new(0));
    let count = effects.clone();
    let (config, handle) = server(move |mut stream| {
        assert_eq!(request(&mut stream)[11..15], [1, 0x14, 1, 0]);
        count.fetch_add(1, Ordering::SeqCst);
        // PLC handled the memory write, then the connection closed before any response.
    });
    let mut client = Client::connect(config).unwrap();
    let error = client.write_m(200, true).unwrap_err();
    assert_eq!(error.stage, FailureStage::ExchangeEntered);
    assert!(error.write_outcome_unknown);
    assert!(client.is_faulted());
    assert!(matches!(
        client.write_m(200, true).unwrap_err().cause,
        Error::Faulted
    ));
    assert!(matches!(
        client.read_m(100, 1).unwrap_err().cause,
        Error::Faulted
    ));
    handle.join().unwrap();
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

#[test]
fn denied_addresses_and_zero_or_oversize_reads_send_no_bytes() {
    let (config, handle) = server(|mut stream| {
        let mut b = [0];
        assert_eq!(stream.read(&mut b).unwrap(), 0);
    });
    let mut client = Client::connect(config).unwrap();
    for (first, count) in [(99, 1), (100, 0), (100, 65), (199, 2), (u32::MAX, 2)] {
        let error = client.read_m(first, count).unwrap_err();
        assert_eq!(error.stage, FailureStage::BeforeSend);
        assert!(!error.write_outcome_unknown);
        assert!(matches!(error.cause, Error::AccessDenied));
    }
    assert!(client.read_d(350, 33).is_err());
    assert!(client.read_d(349, 1).is_err());
    assert!(client.write_m(100, true).is_err());
    assert!(!client.is_faulted());
    drop(client);
    handle.join().unwrap();
}

#[test]
fn plc_error_code_is_preserved_without_assuming_write_nonexecution() {
    let (config, handle) = server(|mut stream| {
        request(&mut stream);
        stream
            .write_all(&[0xd0, 0, 0, 255, 255, 3, 0, 4, 0, 0x51, 0xc0, 0xaa, 0xbb])
            .unwrap();
    });
    let mut client = Client::connect(config).unwrap();
    let error = client.write_m(200, true).unwrap_err();
    assert!(error.write_outcome_unknown);
    match error.cause {
        Error::Plc { code, diagnostic } => {
            assert_eq!(code, 0xc051);
            assert_eq!(diagnostic, [0xaa, 0xbb]);
        }
        e => panic!("{e}"),
    }
    assert!(client.is_faulted());
    handle.join().unwrap();
}

#[test]
fn malformed_headers_lengths_end_codes_and_payloads_fault_the_connection() {
    let good = response(&[0x10]);
    let mut wrong_header = good.clone();
    wrong_header[0] = 0xd4;
    let mut wrong_route = good.clone();
    wrong_route[2] = 1;
    let mut huge = good.clone();
    huge[7..9].copy_from_slice(&65535_u16.to_le_bytes());
    let mut short = good.clone();
    short[7..9].copy_from_slice(&1_u16.to_le_bytes());
    let mut truncated = good.clone();
    truncated.pop();
    let mut missing_end = good.clone();
    missing_end.truncate(10);
    for frame in [
        wrong_header,
        wrong_route,
        huge,
        short,
        truncated,
        missing_end,
        response(&[]),
        response(&[0x10, 0]),
        response(&[0x20]),
        response(&[0x11]),
    ] {
        let (config, handle) = server(move |mut stream| {
            request(&mut stream);
            stream.write_all(&frame).unwrap();
        });
        let mut client = Client::connect(config).unwrap();
        let failure = client.read_m(100, 1).unwrap_err();
        assert!(!failure.write_outcome_unknown);
        assert!(client.is_faulted());
        handle.join().unwrap();
    }
}

#[test]
fn total_deadline_is_not_extended_by_a_slow_trickle() {
    let (mut config, handle) = server(|mut stream| {
        request(&mut stream);
        for b in response(&[0x10]) {
            if stream.write_all(&[b]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
    });
    config.exchange_timeout_ms = 100;
    let mut client = Client::connect(config).unwrap();
    let start = Instant::now();
    assert!(client.read_m(100, 1).is_err());
    // Per-read timeouts would allow all 12 bytes (~330ms). Whole exchange must stop first.
    assert!(start.elapsed() < Duration::from_millis(300));
    assert!(client.is_faulted());
    handle.join().unwrap();
}

#[test]
fn write_ack_payload_cannot_smuggle_a_completion_value() {
    let (config, handle) = server(|mut stream| {
        request(&mut stream);
        stream.write_all(&response(&[1])).unwrap();
    });
    let mut client = Client::connect(config).unwrap();
    let error = client.write_m(200, true).unwrap_err();
    assert!(matches!(error.cause, Error::Protocol(_)));
    assert!(error.write_outcome_unknown);
    handle.join().unwrap();
}

#[test]
fn explicit_route_bytes_and_24_bit_addresses_are_preserved() {
    let (mut config, handle) = server(|mut stream| {
        let frame = request(&mut stream);
        assert_eq!(&frame[2..7], [7, 0x44, 0x34, 0x12, 9]);
        assert_eq!(&frame[15..19], [0x56, 0x34, 0x12, 0x90]);
        let mut reply = response(&[0x10]);
        reply[2..7].copy_from_slice(&[7, 0x44, 0x34, 0x12, 9]);
        stream.write_all(&reply).unwrap();
    });
    config.route = Route {
        network: 7,
        pc: 0x44,
        module_io: 0x1234,
        station: 9,
    };
    config.access.m_last = 0x123456;
    config.access.read_m = vec![AddressRange {
        first: 0x123456,
        last: 0x123456,
    }];
    assert_eq!(
        Client::connect(config)
            .unwrap()
            .read_m(0x123456, 1)
            .unwrap(),
        [true]
    );
    handle.join().unwrap();
}

#[test]
fn profile_is_explicit_bounded_and_simulation_cannot_target_the_site() {
    let good = configuration("127.0.0.1:12345".parse().unwrap());
    good.validate().unwrap();
    for endpoint in [
        "0.0.0.0:1",
        "224.0.0.1:1",
        "255.255.255.255:1",
        "127.0.0.1:0",
        "192.0.2.1:1234",
    ] {
        let mut config = good.clone();
        config.endpoint = endpoint.parse().unwrap();
        assert!(config.validate().is_err());
    }
    let mut physical = good.clone();
    physical.environment = Environment::Physical;
    physical.endpoint = "192.0.2.1:1234".parse().unwrap();
    physical.validate().unwrap(); // Pure validation; never connect to this example address.
    for timer in [0, 21, u16::MAX] {
        let mut config = good.clone();
        config.monitoring_timer = timer;
        assert!(config.validate().is_err());
    }
    for timeout in [0, 5001, u32::MAX] {
        let mut config = good.clone();
        config.exchange_timeout_ms = timeout;
        assert!(config.validate().is_err());
        let mut config = good.clone();
        config.connect_timeout_ms = timeout;
        assert!(config.validate().is_err());
    }
    let mut config = good.clone();
    config.access.m_last = 0x1000000;
    assert!(config.validate().is_err());
    let mut config = good.clone();
    config.access.write_m.insert(1000);
    assert!(config.validate().is_err());
    let mut config = good.clone();
    config.access.read_m.push(AddressRange {
        first: 150,
        last: 160,
    });
    assert!(config.validate().is_err());
    let mut value = serde_json::to_value(good).unwrap();
    value["remote_run"] = true.into();
    assert!(serde_json::from_value::<Configuration>(value).is_err());
}

#[test]
fn invalid_configuration_fails_before_a_connection_attempt() {
    let mut config = configuration("127.0.0.1:12345".parse().unwrap());
    config.access.d_last = 1;
    let Err(error) = Client::connect(config) else {
        panic!("invalid configuration accepted")
    };
    assert_eq!(error.stage, FailureStage::BeforeSend);
    assert!(matches!(error.cause, Error::Configuration(_)));
}

#[test]
fn passive_connect_and_drop_never_send_plc_commands() {
    let (config, handle) = server(|mut stream| {
        let mut b = [0; 128];
        assert_eq!(stream.read(&mut b).unwrap(), 0);
    });
    drop(Client::connect(config).unwrap());
    handle.join().unwrap();
}
