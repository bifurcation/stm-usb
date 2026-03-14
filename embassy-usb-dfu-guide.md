# DFU Runtime with Embassy USB

This guide covers how to add USB DFU runtime support to an Embassy application,
allowing a host to trigger a reboot into the STM32 ROM bootloader over USB —
without any physical BOOT0 pin manipulation.

## Overview

`embassy-usb-dfu` has two operating modes selected by Cargo features:

| Feature       | Mode             | Purpose                                                    |
|---------------|------------------|------------------------------------------------------------|
| `application` | DFU Runtime      | Advertises DFU capability; triggers reboot to bootloader   |
| `dfu`         | DFU Mode         | Full firmware transfer phase; used inside a bootloader     |

For an application that should reboot into the STM32 ROM DFU bootloader, you
want the **`application`** feature.

The `application` feature uses two traits you must implement:

- **`DfuMarker`** — signals that DFU mode is requested on the next boot. We
  implement this by writing a magic value into uninitialised RAM.
- **`Reset`** — initiates the system reset. We implement a custom version that
  first disables the USB peripheral so the host sees a clean disconnect before
  the bootloader re-enumerates.

A `#[cortex_m_rt::pre_init]` hook runs before RAM is zeroed on every boot,
checks for the magic value, and if found jumps directly to the ST ROM
bootloader at `0x1FFF_0000`.

---

## Prerequisites

- `embassy-usb` for the USB stack
- `embassy-usb-dfu` with the `application` feature
- `cortex-m-rt` for the `#[pre_init]` hook
- `cortex-m` for `asm::bootload`

No `embassy-boot` or flash partition setup is required.

---

## Setup

### Cargo.toml

```toml
[dependencies]
embassy-usb     = { version = "0.4", features = ["defmt"] }
embassy-usb-dfu = { version = "0.2", features = ["application"] }
cortex-m        = { version = "0.7", features = ["critical-section-single-core"] }
cortex-m-rt     = "0.7"
static-cell     = "2"
```

### memory.x

No special flash partitions are needed. You only need to ensure the `.uninit`
section is defined so the linker does not zero the magic variable on startup:

```
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 1M
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 128K
}

SECTIONS {
  .uninit (NOLOAD) : {
    *(.uninit .uninit.*);
  } > RAM
}
```

---

## Application Code

### 1. Pre-init hook and magic RAM

This code must be at the top level of your application, before any Embassy
initialisation. The `#[pre_init]` function runs before RAM is initialised, so
the magic value written before the reset is still present.

```rust
use core::mem::MaybeUninit;

// Must match any value you choose — just needs to be unlikely to occur
// naturally in uninitialised RAM.
const MAGIC_JUMP_BOOTLOADER: u32 = 0xDEAD_BEEF;

// STM32F4 system bootloader address — confirm yours in AN2606
const SYSTEM_MEMORY_BASE: u32 = 0x1FFF_0000;

// Placed in .uninit so the startup code does not zero it
#[link_section = ".uninit.MAGIC"]
static mut MAGIC: MaybeUninit<u32> = MaybeUninit::uninit();

/// Runs before RAM initialisation on every boot.
/// If the magic value is present, remaps system memory and jumps to the ST ROM bootloader.
#[cortex_m_rt::pre_init]
unsafe fn check_bootloader_magic() {
    if MAGIC.assume_init() == MAGIC_JUMP_BOOTLOADER {
        MAGIC.as_mut_ptr().write(0);

        // The ROM bootloader expects its own vector table mapped at 0x00000000.
        // Without this remap it crashes silently before USB initialises, which
        // is why the device does not appear on the USB bus at all.

        // Enable SYSCFG clock (RCC_APB2ENR bit 14)
        let rcc_apb2enr = 0x4002_3844 as *mut u32;
        rcc_apb2enr.write_volatile(0x0000_4000);

        // Remap system memory to 0x00000000 (SYSCFG_MEMRMP = 1)
        let syscfg_memrmp = 0x4001_3800 as *mut u32;
        syscfg_memrmp.write_volatile(0x0000_0001);

        // Jump: reads SP from offset +0 and PC from offset +4 of the vector table
        cortex_m::asm::bootload(SYSTEM_MEMORY_BASE as *const u32);
    }
}
```

### 2. Implement `DfuMarker`

`DfuMarker` is the trait `embassy-usb-dfu` calls to signal that DFU mode is
requested. We implement it to write the magic value into RAM:

```rust
use embassy_usb_dfu::application::DfuMarker;

pub struct RomBootloaderMarker;

impl DfuMarker for RomBootloaderMarker {
    fn mark_dfu(&mut self) {
        unsafe {
            MAGIC.as_mut_ptr().write(MAGIC_JUMP_BOOTLOADER);
        }
    }
}
```

### 3. Implement `Reset`

`ResetImmediate` (the built-in) calls `SCB::sys_reset()` straight away. This
does not give the host time to register a USB disconnect, so when the bootloader
re-enumerates the host ignores it. We need a custom `Reset` that disables the
USB peripheral first, waits briefly, then resets.

```rust
use embassy_usb_dfu::application::Reset;

pub struct ResetToBootloader;

impl Reset for ResetToBootloader {
    fn sys_reset(&mut self) -> ! {
        unsafe {
            // Gate the OTG_FS clock via RCC_AHB2ENR (bit 7).
            // This drops D+ low, which the host sees as a cable unplug.
            let rcc_ahb2enr = 0x4002_3834 as *mut u32;
            let val = rcc_ahb2enr.read_volatile();
            rcc_ahb2enr.write_volatile(val & !(1 << 7));

            // Busy-wait ~5 ms at 96 MHz to give the host time to register
            // the disconnect before we reset
            cortex_m::asm::delay(96_000 * 5);
        }

        cortex_m::peripheral::SCB::sys_reset()
    }
}
```

> **Note:** Adjust the `cortex_m::asm::delay` cycle count to match your actual
> CPU frequency. At 100 MHz use `100_000 * 5`, etc.

### 4. Wire it into the USB builder

```rust
use embassy_usb::Builder;
use embassy_usb_dfu::application::{usb_dfu, Control};
use embassy_time::Duration;
use static_cell::StaticCell;

static DFU_HANDLER: StaticCell<Control<RomBootloaderMarker, ResetToBootloader>> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    // --- USB driver setup ---
    let driver = embassy_stm32::usb::Driver::new_fs(
        p.USB_OTG_FS,
        Irqs,
        p.PA12, // D+
        p.PA11, // D-
        &mut ep_out_buffer,
        Default::default(),
    );

    let mut usb_config = embassy_usb::Config::new(0x0483, 0x5740);
    usb_config.manufacturer = Some("My Company");
    usb_config.product = Some("My Device");
    usb_config.serial_number = Some("00000001");

    let mut builder = Builder::new(
        driver,
        usb_config,
        &mut device_descriptor,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buf,
    );

    // --- Add DFU runtime interface ---
    let dfu_handler = DFU_HANDLER.init(
        Control::new(RomBootloaderMarker, ResetToBootloader)
    );

    usb_dfu(
        &mut builder,
        dfu_handler,
        Duration::from_millis(2000), // detach timeout
        |_| {},                      // optional function descriptor modifier
    );

    // Add your other USB classes here (CDC, HID, etc.) ...

    // --- Run USB ---
    let usb = builder.build();
    spawner.spawn(usb_task(usb)).unwrap();
}
```

### 5. Run USB as a background task

```rust
#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, Driver<'static, USB_OTG_FS>>) {
    usb.run().await;
}
```

---

## How It Works

When the host sends a `DFU_DETACH` request followed by a USB reset:

1. `embassy-usb-dfu` calls `RomBootloaderMarker::mark_dfu()`, which writes
   `0xDEAD_BEEF` into the `.uninit` RAM location.
2. It calls `ResetToBootloader::sys_reset()`, which gates the OTG_FS clock to
   drop D+ low, waits ~5 ms for the host to register the disconnect, then
   calls `SCB::sys_reset()`. This clean disconnect is essential — without it
   the host ignores the bootloader's re-enumeration attempt.
3. On the next boot, `check_bootloader_magic()` runs before RAM is zeroed,
   finds the magic value, clears it, enables the SYSCFG clock, remaps system
   memory to `0x00000000`, and calls `cortex_m::asm::bootload()` to jump to
   the ST ROM bootloader at `0x1FFF_0000`. The remap is essential — without it
   the bootloader cannot find its own vector table and crashes before USB starts.
4. The device re-enumerates as a USB DFU device (VID `0483`, PID `df11`).

---

## Triggering from the Host

```bash
# Trigger reboot into bootloader only
dfu-util -e

# Trigger and immediately flash firmware
dfu-util -D firmware.bin
```

After the detach, wait for the device to re-enumerate before flashing. The ST
ROM bootloader appears as VID `0x0483`, PID `0xdf11`.

---

## STM32F4-Specific Notes

- **Bootloader address:** `0x1FFF_0000` for all STM32F4 variants. Confirm yours
  in [AN2606](https://www.st.com/resource/en/application_note/an2606.pdf).
- **BOOT1/PB2:** On the STM32F4, PB2 must be low for the ROM bootloader to
  activate. On Nucleo-144 boards this pin is left floating; close solder bridge
  **SB152** to tie it to GND.
- **USB disconnect before reset:** The host will ignore re-enumeration from the
  bootloader if the application resets without first cleanly dropping the USB
  connection. `ResetToBootloader` handles this by gating the OTG_FS clock to
  pull D+ low before calling `sys_reset()`.
- **SYSCFG memory remap:** The ROM bootloader requires system memory to be
  mapped at `0x00000000` so it can find its own vector table. This is done
  explicitly in `check_bootloader_magic()` before the jump — without it the
  bootloader runs but USB never enumerates.

---

## See Also

- [`embassy-usb-dfu` docs](https://docs.embassy.dev/embassy-usb-dfu)
- [`embassy-usb` docs](https://docs.embassy.dev/embassy-usb)
- [STM32 AN2606 — System Memory Boot Mode](https://www.st.com/resource/en/application_note/an2606.pdf)
