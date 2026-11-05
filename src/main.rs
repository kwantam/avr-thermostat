#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use arduino_hal::{
    hal::wdt,
    prelude::{_embedded_hal_blocking_delay_DelayMs, _unwrap_infallible_UnwrapInfallible},
    Delay,
};
use one_wire_bus::{OneWire, OneWireError};
use panic_halt as _;
use ufmt::uwriteln;

pub mod ds18b20;
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
    match reset::ResetReason::try_from(mcusr) {
        Err(e) => uwriteln!(&mut serial, "RESET unknown({:x})", e),
        Ok(r) => uwriteln!(&mut serial, "RESET {}", <&'static str>::from(r)),
    }
    .unwrap_infallible();

    //  digital output pins
    let mut led = pins.d13.into_output();
    // make it an open drain first to make sure the output buffer is in the right state
    let mut relay = pins.d8.into_opendrain().into_opendrain_high();

    // set up the one-wire thermometer
    let ow_pin = pins.d2.into_opendrain_high();
    let mut ow_bus = OneWire::new(ow_pin).unwrap();
    let mut delay = Delay::new();
    // loop until the thermometer is properly initialized
    let mut thermometer = loop {
        delay.delay_ms(100u16);
        match ds18b20::TempSensor::new(
            &mut ow_bus,
            ds18b20::TempSensorResolution::Bits12,
            None,
            &mut delay,
        ) {
            Err(e) => {
                uwriteln!(&mut serial, "ERR THERM(INIT) {}", ow_error_str(e)).unwrap_infallible()
            }
            Ok(t) => break t,
        };
    };

    // set up the watchdog timer
    let mut watchdog = wdt::Wdt::new(dp.WDT, dp.CPU.mcusr());
    watchdog.start(wdt::Timeout::Ms4000).unwrap_or_default();

    let mut delay = Delay::new();
    let mut state =
        state::ThermostatState::new(&mut led, &mut relay, &mut serial, &mut thermometer);

    loop {
        // reset the watchdog timer
        watchdog.feed();

        // INVARIANT: state.temperature is None at this point
        assert!(state.take_temperature().is_none());

        // if we're converting, wait until conversion is complete and then read the value
        state.update_temperature();

        // immediately start a new conversion
        state.start_conversion();

        // Check if we have a temperature reading and maybe change thermostat state if so
        if let Some(t) = state.take_temperature() {
            // good temperature reading, so clear any temp sensor error count
            state.clear_temp_err();
            state.write_temperature(t);

            // decide whether to change state
            if state.cooling() {
                unimplemented!()
            } else {
                unimplemented!()
            }
        } else if state.is_temp_err() {
            // if we're in temperature sensor error state, turn off the thermostat...
            state.thermostat_off();
            // ...then go into a busyloop until wdt resets us
            loop {
                delay.delay_ms(1000u16);
            }
        }
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
