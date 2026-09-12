use crate::{
    AccessProfile, Configuration, Error, Result,
    codec::{self, Request},
};
use std::{
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureStage {
    /// Input/access/connection failure before a write syscall was attempted.
    BeforeSend,
    /// Bytes may have reached the PLC. This says nothing about physical execution.
    ExchangeEntered,
}
#[derive(Debug, thiserror::Error)]
#[error("MC exchange failed at {stage:?}: {cause}")]
pub struct Failure {
    pub stage: FailureStage,
    pub write_outcome_unknown: bool,
    #[source]
    pub cause: Error,
}
impl Failure {
    fn before(cause: Error) -> Self {
        Self {
            stage: FailureStage::BeforeSend,
            write_outcome_unknown: false,
            cause,
        }
    }
}
/// An exact end-code-zero MC response only. Never use this as physical completion evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteAcknowledgement {
    pub address: u32,
    pub requested_value: bool,
}

/// One TCP connection, one outstanding request. Any exchange error permanently faults it.
/// Recreating a Client does not authorize resubmission of an earlier operation.
pub struct Client {
    config: Configuration,
    stream: TcpStream,
    faulted: bool,
}
impl Client {
    /// Opens TCP only. It neither probes addresses nor sends a request.
    pub fn connect(config: Configuration) -> std::result::Result<Self, Failure> {
        config.validate().map_err(Failure::before)?;
        let stream = TcpStream::connect_timeout(
            &SocketAddr::V4(config.endpoint),
            Duration::from_millis(config.connect_timeout_ms.into()),
        )
        .map_err(|e| Failure::before(e.into()))?;
        stream
            .set_nodelay(true)
            .map_err(|e| Failure::before(e.into()))?;
        Ok(Self {
            config,
            stream,
            faulted: false,
        })
    }
    pub fn is_faulted(&self) -> bool {
        self.faulted
    }

    pub fn read_m(&mut self, first: u32, count: u16) -> std::result::Result<Vec<bool>, Failure> {
        if !AccessProfile::permits(&self.config.access.read_m, first, count, 64) {
            return Err(Failure::before(Error::AccessDenied));
        }
        let data = self.exchange(Request::ReadM { first, count })?;
        codec::bits(&data, count).map_err(|cause| self.fail(cause, false))
    }
    pub fn read_d(&mut self, first: u32, count: u16) -> std::result::Result<Vec<u16>, Failure> {
        if !AccessProfile::permits(&self.config.access.read_d, first, count, 32) {
            return Err(Failure::before(Error::AccessDenied));
        }
        let data = self.exchange(Request::ReadD { first, count })?;
        Ok(data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect())
    }
    /// Caller must durably record SEND_ENTERED and check current native authority first.
    /// Neither this method nor any read reconnects or retries this write.
    pub fn write_m(
        &mut self,
        address: u32,
        value: bool,
    ) -> std::result::Result<WriteAcknowledgement, Failure> {
        if !self.config.access.write_m.contains(&address) {
            return Err(Failure::before(Error::AccessDenied));
        }
        self.exchange(Request::WriteM { address, value })?;
        Ok(WriteAcknowledgement {
            address,
            requested_value: value,
        })
    }
    fn fail(&mut self, cause: Error, is_write: bool) -> Failure {
        self.faulted = true;
        let _ = self.stream.shutdown(Shutdown::Both);
        Failure {
            stage: FailureStage::ExchangeEntered,
            write_outcome_unknown: is_write,
            cause,
        }
    }
    fn exchange(&mut self, request: Request) -> std::result::Result<Vec<u8>, Failure> {
        if self.faulted {
            return Err(Failure::before(Error::Faulted));
        }
        let deadline = Instant::now() + self.config.exchange_timeout();
        let frame = request.encode(self.config.route, self.config.monitoring_timer);
        let outcome = (|| -> Result<Vec<u8>> {
            write_until(&mut self.stream, &frame, deadline)?;
            let mut header = [0_u8; 9];
            read_until(&mut self.stream, &mut header, deadline)?;
            let len = codec::response_length(&header, self.config.route)?;
            let mut body = vec![0; len];
            read_until(&mut self.stream, &mut body, deadline)?;
            let payload = codec::payload(&body, request.response_len())?;
            Ok(payload.to_vec())
        })();
        outcome.map_err(|cause| self.fail(cause, request.is_write()))
    }
}
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "MC total exchange deadline"))
}
fn read_until(stream: &mut TcpStream, mut buffer: &mut [u8], deadline: Instant) -> io::Result<()> {
    while !buffer.is_empty() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(buffer) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated MC response",
                ));
            }
            Ok(n) => {
                buffer = &mut buffer[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    remaining(deadline)?;
    Ok(())
}
fn write_until(stream: &mut TcpStream, mut buffer: &[u8], deadline: Instant) -> io::Result<()> {
    while !buffer.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(buffer) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "MC request write stopped",
                ));
            }
            Ok(n) => {
                buffer = &buffer[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    remaining(deadline)?;
    Ok(())
}
