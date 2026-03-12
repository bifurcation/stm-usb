# STM-USB

WebUSB echo device firmware for STM32F4xx with a web-based control panel.

## Project Structure

- **firmware/** - Embassy-based STM32F4xx firmware (supports STM32F411CE and STM32F412ZG)
- **control/** - WASM web application for flashing and communication

## Requirements

- Rust with targets: `rustup target add thumbv7em-none-eabihf wasm32-unknown-unknown`
- cargo-binutils: `cargo install cargo-binutils && rustup component add llvm-tools`
- wasm-pack: `cargo install wasm-pack`
- Python 3 (for serving)

## Quick Start

Build everything and start the server:

```bash
make serve
```

Open http://localhost:8080 in Chrome or Edge (WebUSB requires a Chromium-based browser).

## Make Targets

| Target               | Description                                      |
|----------------------|--------------------------------------------------|
| `make firmware`      | Build firmware and create DFU binary (.bin)      |
| `make wasm`          | Build WASM control panel (requires firmware first) |
| `make serve`         | Build all and serve on http://localhost:8080     |
| `make clean`         | Clean all build artifacts                        |
| `make check`         | Run formatting and build checks (same as CI)     |
| `make install-hooks` | Install git pre-push hook                        |

## Usage

### Flashing Firmware via DFU

1. Put the STM32F411 into DFU mode:
   - Hold BOOT0 button
   - Press and release RESET
   - Release BOOT0
2. Click "Flash Firmware via DFU" in the web UI
3. Select the DFU device when prompted
4. Wait for flashing to complete

### Communicating with the Device

1. After flashing, wait a few seconds for the device to reboot
2. Click "Connect to Device"
3. Select the device when prompted
4. Type text and press Enter or click Send
5. The device echoes back each line

## Hardware Assumptions

This firmware has not been tested on real hardware. It assumes:

| Assumption            | Value                       | Code Location                                       |
|-----------------------|-----------------------------|-----------------------------------------------------|
| MCU                   | STM32F411CE or STM32F412ZG  | [Makefile:8](Makefile#L8) (`CHIP` variable)         |
| HSE crystal frequency | 25 MHz                      | [firmware/src/main.rs:34](firmware/src/main.rs#L34) |
| USB D- pin            | PA11                        | [firmware/src/main.rs:64](firmware/src/main.rs#L64) |
| USB D+ pin            | PA12                        | [firmware/src/main.rs:63](firmware/src/main.rs#L63) |
| VBUS detection        | Disabled                    | [firmware/src/main.rs:58](firmware/src/main.rs#L58) |

To build for a different chip:

```bash
make firmware CHIP=stm32f412
```

## USB IDs

| Mode        | VID    | PID    |
|-------------|--------|--------|
| Application | 0x1209 | 0x0001 |
| DFU         | 0x0483 | 0xDF11 |
