use super::*;
#[cfg(target_os = "linux")]
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
pub(super) fn ping(
    directory: &Path,
    pin: Digest,
    request: &serde_json::Value,
) -> Result<serde_json::Value> {
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    let bytes = rx_package::directory::read_relative_file(
        Path::new("/opt/rx/bin"),
        &rx_package::PackagePath::new("rx-dynamixel-ping").map_err(unknown)?,
        16 * 1024 * 1024,
    )
    .map_err(unknown)?;
    if rx_package::content_digest(&bytes) != pin {
        return Err(unknown("DXL_HELPER_PIN_MISMATCH"));
    }
    let (mut channel, child_channel) = UnixStream::pair().map_err(unknown)?;
    channel
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(unknown)?;
    channel
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(unknown)?;
    let audit = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("helper-invocations.jsonl"))
        .map_err(unknown)?;
    let mut child = Command::new(profile::BINARY)
        .args(["--endpoint", profile::ENDPOINT])
        .env_clear()
        .stdin(Stdio::from(OwnedFd::from(child_channel)))
        .stdout(Stdio::null())
        .stderr(Stdio::from(audit))
        .spawn()
        .map_err(unknown)?;
    let result = (|| {
        let mut input = rx_domain::canonical::bytes(request).map_err(unknown)?;
        input.push(b'\n');
        channel.write_all(&input).map_err(unknown)?;
        let mut reply = Vec::new();
        let mut byte = [0];
        while reply.len() < 4096 {
            if channel.read(&mut byte).map_err(unknown)? == 0 {
                break;
            }
            if byte[0] == b'\n' {
                break;
            }
            reply.push(byte[0]);
        }
        if reply.len() >= 4096 {
            return Err(unknown("DXL_REPLY_TOO_LARGE"));
        }
        let value: serde_json::Value =
            rx_domain::canonical::decode_json(&reply).map_err(unknown)?;
        let end = Instant::now() + Duration::from_millis(200);
        loop {
            if let Some(status) = child.try_wait().map_err(unknown)? {
                if !status.success() {
                    return Err(unknown("DXL_HELPER_EXIT"));
                }
                break;
            }
            if Instant::now() >= end {
                return Err(unknown("DXL_HELPER_EXIT_DEADLINE"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if value["request"] != *request
            || value["schema"] != "rx.dynamixel.ping-capture.v1"
            || value["sdk"] != "4.1.0"
            || value["transport"] != "SIMULATED"
            || value["model"] != 65500
            || value["protocol"] != 2
            || value["device_id"] != 1
            || value["result"] != 0
            || value["error"] != 0
            || value["writes"] != 1
            || value["tx"] != "fffffd0001030001194e"
        {
            return Err(unknown("DXL_PING_UNCONFIRMED"));
        }
        Ok(value)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
#[cfg(not(target_os = "linux"))]
pub(super) fn ping(_: &Path, _: Digest, _: &serde_json::Value) -> Result<serde_json::Value> {
    Err(unknown("DXL_LINUX_HOST_CHANNEL_REQUIRED"))
}
