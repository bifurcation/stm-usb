# STM-USB

WebUSB echo device firmware for STM32F411CEU6 with a web-based control panel.

## Project Structure

- **firmware/** - Embassy-based STM32F411CEU6 firmware
- **control/** - WASM web application for flashing and communication

## Requirements

### Firmware
- Rust with `thumbv7em-none-eabihf` target: `rustup target add thumbv7em-none-eabihf`
- probe-rs (optional, for debugging): `cargo install probe-rs-tools`

### Control
- wasm-pack: `cargo install wasm-pack`
- A web server (e.g., `python3 -m http.server` or `npx serve`)

## Building

### Firmware

```bash
cd firmware
cargo build --release
```

The binary will be at `target/thumbv7em-none-eabihf/release/firmware`.

### Control Panel

The control crate embeds the firmware binary, so build the firmware first.

```bash
cd control
wasm-pack build --target web --out-dir www/pkg
```

## Running

Serve the control panel:

```bash
cd control/www
python3 -m http.server 8080
```

Open http://localhost:8080 in Chrome or Edge (WebUSB requires a Chromium-based browser).

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

## Hardware

Tested with STM32F411CEU6 ("Black Pill" board) with 25MHz crystal.

USB pins:
- PA11 - USB D-
- PA12 - USB D+

## USB IDs

| Mode | VID | PID |
|------|-----|-----|
| Application | 0x1209 | 0x0001 |
| DFU | 0x0483 | 0xDF11 |
