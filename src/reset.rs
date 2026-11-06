//! Detect reset reason from MCUSR

pub enum ResetReason {
    Watchdog,
    Brownout,
    External,
    PowerOn,
}

impl TryFrom<u8> for ResetReason {
    type Error = u8;

    fn try_from(other: u8) -> Result<Self, Self::Error> {
        match other {
            0x08 => Ok(Self::Watchdog),
            0x04 => Ok(Self::Brownout),
            0x02 => Ok(Self::External),
            0x01 => Ok(Self::PowerOn),
            e => Err(e),
        }
    }
}

impl From<ResetReason> for &'static str {
    fn from(reason: ResetReason) -> &'static str {
        match reason {
            ResetReason::Watchdog => "Watchdog",
            ResetReason::Brownout => "Brownout",
            ResetReason::External => "External",
            ResetReason::PowerOn => "PowerOn",
        }
    }
}

macro_rules! write_reset_reason {
    ($serial:expr, $mcusr:expr) => {
        match reset::ResetReason::try_from($mcusr) {
            Err(e) => uwriteln!(&mut $serial, "0 RESET unknown({:x})", e),
            Ok(r) => uwriteln!(&mut $serial, "0 RESET {}", <&'static str>::from(r)),
        }
        .unwrap_infallible();
    };
}
