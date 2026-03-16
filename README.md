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

### WeAct Black Pill (STM32F411CE)

The Black Pill can be flashed entirely from the browser using the ROM bootloader.

**First flash:**
1. Start the server:
   ```bash
   make serve-f411
   ```
2. Open http://localhost:8080 in Chrome/Edge
3. Put the board into DFU mode:
   - Hold BOOT0 button
   - Press and release NRST (reset)
   - Release BOOT0
4. Scroll to "First Flash" and click **Direct Flash**
5. Select "STM32 BOOTLOADER" when prompted

**Subsequent flashes:** Connect and click Flash (the firmware resets to DFU automatically).

### NUCLEO-F412ZG

The NUCLEO board requires ST-LINK for the first flash (probe-rs must be installed).

**First flash:**
```bash
cd firmware && cargo run
```

**Subsequent flashes** (via web UI):
```bash
make serve
```
Open http://localhost:8080, connect to the device, and click Flash.

## Make Targets

| Target               | Description                                      |
|----------------------|--------------------------------------------------|
| `make serve`         | Build for F412 (NUCLEO) and serve web UI         |
| `make serve-f411`    | Build for F411 (Black Pill) and serve web UI     |
| `make firmware`      | Build firmware only (uses CHIP variable)         |
| `make wasm`          | Build WASM control panel                         |
| `make clean`         | Clean all build artifacts                        |
| `make check`         | Run formatting and build checks (same as CI)     |
| `make install-hooks` | Install git pre-push hook                        |

## Communicating with the Device

1. After flashing, wait for the device to reboot
2. Click **Connect** in the web UI
3. Select the device when prompted
4. Type text and press Enter or click Send
5. The device echoes back with "ECHO " prefix

## Hardware

### Supported Boards

| Board                  | Chip         | HSE    | Notes                           |
|------------------------|--------------|--------|---------------------------------|
| NUCLEO-F412ZG          | STM32F412ZG  | 8 MHz  | ST-LINK bypass mode             |
| WeAct Black Pill V3.1  | STM32F411CE  | 25 MHz | No debugger, DFU flash only     |

### Pin Assignments

| Function       | Pin  |
|----------------|------|
| USB D-         | PA11 |
| USB D+         | PA12 |
| LED (F411)     | PC13 |
| LED (F412)     | PB14 |

VBUS detection is disabled (assumes always connected).

## USB IDs

| Mode        | VID    | PID    |
|-------------|--------|--------|
| Application | 0x1209 | 0x0001 |
| DFU         | 0x0483 | 0xDF11 |
