use crate::{Error, Result, Route};

pub(crate) enum Request {
    ReadM { first: u32, count: u16 },
    ReadD { first: u32, count: u16 },
    WriteM { address: u32, value: bool },
}
impl Request {
    pub(crate) fn encode(&self, route: Route, timer: u16) -> Vec<u8> {
        let (command, subcommand, first, code, count): (u16, u16, u32, u8, u16) = match *self {
            Self::ReadM { first, count } => (0x0401, 1, first, 0x90, count),
            Self::ReadD { first, count } => (0x0401, 0, first, 0xa8, count),
            Self::WriteM { address, .. } => (0x1401, 1, address, 0x90, 1),
        };
        let length = if matches!(self, Self::WriteM { .. }) {
            13_u16
        } else {
            12
        };
        let mut frame = Vec::with_capacity(9 + usize::from(length));
        frame.extend([0x50, 0]);
        frame.extend(route.bytes());
        frame.extend(length.to_le_bytes());
        frame.extend(timer.to_le_bytes());
        frame.extend(command.to_le_bytes());
        frame.extend(subcommand.to_le_bytes());
        frame.extend(&first.to_le_bytes()[..3]);
        frame.push(code);
        frame.extend(count.to_le_bytes());
        if let Self::WriteM { value, .. } = self {
            frame.push(if *value { 0x10 } else { 0 });
        }
        frame
    }
    pub(crate) fn response_len(&self) -> usize {
        match *self {
            Self::ReadM { count, .. } => usize::from(count).div_ceil(2),
            Self::ReadD { count, .. } => usize::from(count) * 2,
            Self::WriteM { .. } => 0,
        }
    }
    pub(crate) fn is_write(&self) -> bool {
        matches!(self, Self::WriteM { .. })
    }
}

pub(crate) fn response_length(header: &[u8; 9], route: Route) -> Result<usize> {
    if header[..2] != [0xd0, 0] || header[2..7] != route.bytes() {
        return Err(Error::Protocol("subheader or routing mismatch"));
    }
    let len = usize::from(u16::from_le_bytes([header[7], header[8]]));
    // Max successful payload is 32 words; retain bounded PLC error diagnostics too.
    if !(2..=258).contains(&len) {
        return Err(Error::Protocol("response length outside bound"));
    }
    Ok(len)
}
pub(crate) fn payload(body: &[u8], expected: usize) -> Result<&[u8]> {
    if body.len() < 2 {
        return Err(Error::Protocol("missing end code"));
    }
    let code = u16::from_le_bytes([body[0], body[1]]);
    if code != 0 {
        return Err(Error::Plc {
            code,
            diagnostic: body[2..].to_vec(),
        });
    }
    if body.len() != expected + 2 {
        return Err(Error::Protocol("unexpected success payload size"));
    }
    Ok(&body[2..])
}
pub(crate) fn bits(data: &[u8], count: u16) -> Result<Vec<bool>> {
    if data.len() != usize::from(count).div_ceil(2) {
        return Err(Error::Protocol("bit payload length"));
    }
    let mut values = Vec::with_capacity(usize::from(count));
    for byte in data {
        for bit in [byte >> 4, byte & 0xf] {
            if values.len() == usize::from(count) {
                // An unused half-byte cannot provide another device value.
                if bit != 0 {
                    return Err(Error::Protocol("nonzero bit padding"));
                }
            } else {
                match bit {
                    0 => values.push(false),
                    1 => values.push(true),
                    _ => return Err(Error::Protocol("bit value is neither zero nor one")),
                }
            }
        }
    }
    Ok(values)
}
