#![no_std]
#![no_main]

use arduino_hal::{
    hal::wdt,
    prelude::{_embedded_hal_blocking_delay_DelayMs, _unwrap_infallible_UnwrapInfallible},
    Delay,
};
use one_wire_bus::{OneWire, OneWireError};
use panic_halt as _;
use ufmt::uwriteln;

#[macro_use]
pub mod ds18b20;
#[macro_use]
pub mod reset;
pub mod state;

type OwError = OneWireError<core::convert::Infallible>;

#[arduino_hal::entry]
fn main() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    // check reset reason
    let mcusr = dp.CPU.mcusr().read().bits();
    dp.CPU.mcusr().reset();

    // set up serial port and send reset reason
    let mut serial = arduino_hal::default_serial!(dp, pins, 38400);
    write_reset_reason!(serial, mcusr);

    // status LED
    let mut led = pins.d13.into_output();
    // relay
    // make it an open drain first to make sure the output buffer is in the right state
    let mut relay = pins.d8.into_opendrain().into_opendrain_high();

    // one-wire bus
    let ow_pin = pins.d2.into_opendrain_high();
    let mut ow_bus = OneWire::new(ow_pin).unwrap();
    let mut delay = Delay::new();

    // initialize the temperature sensor
    let mut count = 1u16;
    let mut thermometer = temp_sensor!(
        ow_bus,
        ds18b20::TempSensorResolution::Bits12,
        None,
        delay,
        serial,
        &mut count
    );

    // construct the state machine controller
    let mut state =
        state::ThermostatState::new(&mut led, &mut relay, &mut serial, &mut thermometer, count);

    // set up the watchdog timer
    let mut watchdog = wdt::Wdt::new(dp.WDT, dp.CPU.mcusr());
    watchdog.start(wdt::Timeout::Ms4000).unwrap_or_default();

    loop {
        // reset the watchdog timer
        watchdog.feed();

        // if we're converting, wait until conversion is complete and then read the value
        state.update_temperature();

        // immediately start a new conversion
        state.start_conversion();

        // update compressor state if indicated
        state.update_cooling_state();

        // increment the debouncer for min on- and off-time
        state.increment_debounce_cnt();
    }
}

fn ow_error_str<E>(e: OneWireError<E>) -> &'static str {
    match e {
        OneWireError::BusNotHigh => "OWE BusNotHigh",
        OneWireError::PinError(_) => "OWE PinError",
        OneWireError::UnexpectedResponse => "OWE UnexpectedResponse",
        OneWireError::FamilyCodeMismatch => "OWE FamilyCodeMismatch",
        OneWireError::CrcMismatch => "OWE CrcMismatch",
        OneWireError::Timeout => "OWE Timeout",
    }
}
