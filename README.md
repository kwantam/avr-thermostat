avr-thermostat
==============

This is an arduino nano--based thermostat that I designed to control
a True GDM-10PT refrigerator. It will almost certainly work with your
refrigerator, too.

The temperature setpoints are hard coded; see `src/state.rs`. If you want
to change them, recompile and re-flash. The microcontroller has plenty of
spare inputs, so it wouldn't be all that hard to make things programmable.

Connections: `src/main.rs` encodes which pins correspond to which peripherals.
Those peripherals are:

- An optocoupler driving a relay, active-low driven by open drain. Make
  sure the relay can handle the current!

- A DS18B20 temperature sensor. These are widely available and inexpensive,
  and they give quite reasonable accuracy (1/2 degree C).

## How to hook it up

Ideally you'd have the temperature sensor submerged in water in the fridge.
If you want to control the temperature of, say, a carboy, strap it to the
side of the carboy or get a thermometer well.

For the relay, you have two general options:

- Use the relay to control power to the fridge. The relay turns the entire
  fridge on and off. Make sure the thermostat in the fridge is set no warmer
  than the trip points in your code or it will interfere with operation.

- Use the relay in place of the current thermostat in the fridge. Most
  fridges have a temperature-sensing mechanical switch that closes when
  warm (allowing the compressor to run) and opens when cold.

  You might consider instead putting thre relay in series with the mechanical
  thermostat. This would prevent the fridge contents from freezing if the
  relay welds itself shut (which is rare but can happen). It would also
  give you any defrost or other behavior.

## Serial console

The namo writes status events to the serial port. Eventually, these will
be documented in detail here.

Connect this to a computer, open the USB-serial port at 38400 8N1. You
can do this with `ravedude -c`, with `screen /dev/usb-serial 38400`,
or with your favorite serial console.

One thing that will probably make your life easier is removing C4 from
the Arduino nano board. That capacitor connects the DTR pin from the
USB-serial chip to the Arduino's reset line. Note that if you do this,
you must program your board with something like a usbasp; programming
through the USB port probably doesn't work with this capacitor removed.

## Build Instructions
1. Install prerequisites as described in the [`avr-hal` README] (`avr-gcc`, `avr-libc`, `avrdude`, [`ravedude`]).

2. Run `cargo build` to build the firmware.

3. Run `cargo run` to flash the firmware to a connected board.  If `ravedude`
   fails to detect your board, check its documentation at
   <https://crates.io/crates/ravedude>.
   
   NOTE: This project is configured to program with a usbasp programmer.

4. Connect the USB port and open a serial console via `ravedude -c`.

[`avr-hal` README]: https://github.com/Rahix/avr-hal#readme
[`ravedude`]: https://crates.io/crates/ravedude

## License
Licensed under either of

 - Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 - MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
