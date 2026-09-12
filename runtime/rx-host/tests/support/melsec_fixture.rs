use rx_domain::{intent::*, types::*};
use rx_host::{melsec::*, native::*, simulation::ManualClock, *};
use std::{
    io::{Read, Write},
    net::{SocketAddrV4, TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
pub fn name(v: &str) -> Name {
    Name::new(v).unwrap()
}
pub fn id() -> Id {
    Id::new(uuid::Uuid::new_v4().to_string()).unwrap()
}
pub fn digest(v: u8) -> Digest {
    Digest::from_bytes([v; 32])
}
pub fn clock() -> ManualClock {
    ManualClock {
        clock_id: "simulation/melsec".into(),
        ticks: Arc::new(AtomicU64::new(1_000_000_000)),
    }
}
// Serial test ownership prevents unrelated process-spawn fork windows from inheriting another test's briefly-held initialization lock. Product lock checks remain strict.
pub static TEST_SERIAL: Mutex<()> = Mutex::new(());
pub const READY: u16 = 0b11_1111;
pub const COMPLETE: u16 = 1 << 6;
#[derive(Default)]
pub struct PlcState {
    pub epoch: u64,
    pub sequence: u64,
    pub flags: u16,
    pub writes: Vec<(u32, bool)>,
    pub reads: usize,
    pub freeze: bool,
    pub lose_reply: bool,
    pub complete_on_write: bool,
    pub advance_clock_on_read: Option<Arc<AtomicU64>>,
}
pub struct Plc {
    pub endpoint: SocketAddrV4,
    pub state: Arc<Mutex<PlcState>>,
    pub stop: Arc<AtomicBool>,
    pub handle: Option<thread::JoinHandle<()>>,
}
impl Plc {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let std::net::SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            panic!()
        };
        let state = Arc::new(Mutex::new(PlcState {
            epoch: 1,
            sequence: 1,
            flags: READY,
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let st = state.clone();
        let quit = stop.clone();
        let handle = thread::spawn(move || {
            let mut workers = Vec::new();
            while !quit.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let st = st.clone();
                        let quit = quit.clone();
                        workers.push(thread::spawn(move || serve(stream, st, quit)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            for w in workers {
                w.join().unwrap();
            }
        });
        Self {
            endpoint,
            state,
            stop,
            handle: Some(handle),
        }
    }
    pub fn update(&self, f: impl FnOnce(&mut PlcState)) {
        f(&mut self.state.lock().unwrap());
    }
    pub fn writes(&self) -> usize {
        self.state.lock().unwrap().writes.len()
    }
}
impl Drop for Plc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take().unwrap().join().unwrap();
    }
}
fn serve(mut stream: TcpStream, state: Arc<Mutex<PlcState>>, stop: Arc<AtomicBool>) {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    while !stop.load(Ordering::SeqCst) {
        let mut header = [0; 9];
        match stream.read_exact(&mut header) {
            Ok(()) => (),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return,
        }
        let len = usize::from(u16::from_le_bytes([header[7], header[8]]));
        assert!((12..=13).contains(&len));
        let mut body = vec![0; len];
        if stream.read_exact(&mut body).is_err() {
            return;
        }
        assert_eq!(&header[..7], &[0x50, 0, 0, 255, 255, 3, 0]);
        let command = u16::from_le_bytes([body[2], body[3]]);
        let address = u32::from_le_bytes([body[6], body[7], body[8], 0]);
        let mut payload = Vec::new();
        {
            let mut st = state.lock().unwrap();
            if command == 0x0401 {
                assert_eq!((address, body[9], body[10], body[11]), (350, 0xa8, 9, 0));
                st.reads += 1;
                if let Some(clock) = &st.advance_clock_on_read {
                    clock.fetch_add(40_000_000, Ordering::SeqCst);
                }
                if !st.freeze {
                    st.sequence += 1;
                }
                payload.extend(st.epoch.to_le_bytes());
                payload.extend(st.sequence.to_le_bytes());
                payload.extend(st.flags.to_le_bytes());
            } else {
                assert_eq!(
                    (command, address, body[9], body[10], body[11]),
                    (0x1401, 200, 0x90, 1, 0)
                );
                assert!(matches!(body[12], 0 | 0x10));
                st.writes.push((address, body[12] == 0x10));
                if st.complete_on_write {
                    st.flags |= COMPLETE;
                }
                // Memory write ACK deliberately does not change completion or support flags.
                if st.lose_reply {
                    return;
                }
            }
        }
        let mut reply = vec![0xd0, 0, 0, 255, 255, 3, 0];
        reply.extend(u16::try_from(payload.len() + 2).unwrap().to_le_bytes());
        reply.extend([0, 0]);
        reply.extend(payload);
        if stream.write_all(&reply).is_err() {
            return;
        }
    }
}
pub fn profile(plc: &Plc) -> Profile {
    Profile {
        schema: name("rx.melsec-predicate-profile.v1"),
        installation: id(),
        cell: name("cell/laser"),
        target: name("laser/chuck"),
        site_config: digest(2),
        calibrations: vec![],
        resources: vec![name("laser/controller")],
        transport: rx_melsec_mc::Configuration {
            environment: rx_melsec_mc::Environment::Simulation,
            endpoint: plc.endpoint,
            route: rx_melsec_mc::Route {
                network: 0,
                pc: 255,
                module_io: 0x03ff,
                station: 0,
            },
            monitoring_timer: 1,
            connect_timeout_ms: 50,
            exchange_timeout_ms: 30,
            access: rx_melsec_mc::AccessProfile {
                m_last: 1000,
                d_last: 1000,
                read_m: vec![],
                read_d: vec![rx_melsec_mc::AddressRange {
                    first: 350,
                    last: 358,
                }],
                write_m: [200].into(),
            },
        },
        status: StatusImage {
            first_d: 350,
            publication_contract: digest(3),
            plc_program: digest(4),
            source_max_age_ms: 30,
            read_budget_ms: 50,
            guard_validity_ms: 30,
            valid_bit: 0,
            ready_bit: 1,
            queue_empty_bit: 2,
            control_bit: 3,
            support_bit: 4,
            drop_allowed_bit: 5,
        },
        conditions: [(name("laser/ready"), 1)].into(),
        sources: [(name("laser/ready"), 1), (name("chuck/closed"), 6)].into(),
        predicates: vec![PredicateMapping {
            predicate: name("chuck.closed"),
            command_m: 200,
            completion_bit: 6,
            settle_ms: Counter(100),
        }],
    }
}
pub fn intent(p: &Profile) -> Intent {
    Intent {
        kind: Kind::EnsureState,
        target: p.target.clone(),
        profile_digest: p.digest().unwrap(),
        site_config_digest: p.site_config,
        calibration_digests: p.calibrations.clone(),
        resource_set: p.resources.clone(),
        execution_timeout_ms: Counter(5000),
        prepare_validity_ms: Counter(1000),
        completion_rule: name("rx.melsec.debounced-predicate.v1"),
        cancel_rule: name("rx.melsec.no-native-cancel.v1"),
        body: Body::Predicate(PredicateGoal {
            predicate_id: name("chuck.closed"),
            target: TypedValue::Boolean(true),
            settle_ms: Counter(100),
        }),
    }
}
pub fn open(directory: &Path, p: &Profile, c: &ManualClock) -> (Identity, Melsec<ManualClock>) {
    let identity = Melsec::<ManualClock>::initialize(directory, p).unwrap();
    let device = Melsec::open(directory, &identity, p.clone(), c.clone()).unwrap();
    (identity, device)
}
pub fn warm<H: NativeBoundary>(device: &Melsec<ManualClock, H>, p: &Profile, c: &ManualClock) {
    assert!(device.guard(&intent(p), &c.now()).is_err());
    device.guard(&intent(p), &c.now()).unwrap();
}
