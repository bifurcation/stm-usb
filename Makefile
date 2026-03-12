# Directories
ROOT_DIR     := $(shell pwd)
FIRMWARE_DIR := $(ROOT_DIR)/firmware
CONTROL_DIR  := $(ROOT_DIR)/control
WWW_DIR      := $(CONTROL_DIR)/www

# Chip selection (stm32f411 or stm32f412)
CHIP := stm32f412

# Target triple
TARGET := thumbv7em-none-eabihf

# Output files
FIRMWARE_ELF := $(FIRMWARE_DIR)/target/$(TARGET)/release/firmware
FIRMWARE_BIN := $(FIRMWARE_ELF).bin
WWW_FIRMWARE := $(WWW_DIR)/firmware.bin
WASM_PKG     := $(WWW_DIR)/pkg/control.js

# Source files
FIRMWARE_SRC := $(wildcard $(FIRMWARE_DIR)/src/*.rs) $(FIRMWARE_DIR)/Cargo.toml
CONTROL_SRC  := $(wildcard $(CONTROL_DIR)/src/*.rs) $(CONTROL_DIR)/Cargo.toml

.PHONY: all firmware wasm serve clean check install-hooks

all: firmware wasm

firmware: $(FIRMWARE_BIN)
wasm: $(WASM_PKG)

$(FIRMWARE_BIN): $(FIRMWARE_SRC)
	cd $(FIRMWARE_DIR) && cargo build --release --features $(CHIP)
	cd $(FIRMWARE_DIR) && cargo objcopy --release --features $(CHIP) -- -O binary $(FIRMWARE_BIN)

$(WASM_PKG): $(CONTROL_SRC)
	cd $(CONTROL_DIR) && wasm-pack build --target web --out-dir $(WWW_DIR)/pkg

$(WWW_FIRMWARE): $(FIRMWARE_BIN)
	cp $(FIRMWARE_BIN) $(WWW_FIRMWARE)

serve: wasm $(WWW_FIRMWARE)
	python3 -m http.server 8080 --directory $(WWW_DIR)

# Always run wasm-pack (it's smart enough to skip if nothing changed)
.PHONY: wasm-force
wasm-force:
	cd $(CONTROL_DIR) && wasm-pack build --target web --out-dir $(WWW_DIR)/pkg

clean:
	cd $(FIRMWARE_DIR) && cargo clean
	cd $(CONTROL_DIR) && cargo clean
	rm -rf $(WWW_DIR)/pkg $(WWW_FIRMWARE)

check:
	./scripts/check.sh

install-hooks:
	ln -sf ../../scripts/pre-push .git/hooks/pre-push
