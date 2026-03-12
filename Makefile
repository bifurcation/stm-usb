# Directories
FIRMWARE_DIR := firmware
CONTROL_DIR := control
WWW_DIR := $(CONTROL_DIR)/www

# Target triple
TARGET := thumbv7em-none-eabihf

# Output files
FIRMWARE_ELF := $(FIRMWARE_DIR)/target/$(TARGET)/release/firmware
FIRMWARE_BIN := $(FIRMWARE_ELF).bin
WWW_FIRMWARE := $(WWW_DIR)/firmware.bin
WASM_PKG := $(WWW_DIR)/pkg/control.js

# Source files
FIRMWARE_SRC := $(wildcard $(FIRMWARE_DIR)/src/*.rs) $(FIRMWARE_DIR)/Cargo.toml
CONTROL_SRC := $(wildcard $(CONTROL_DIR)/src/*.rs) $(CONTROL_DIR)/Cargo.toml

.PHONY: all firmware wasm serve clean check install-hooks

all: firmware wasm

firmware: $(FIRMWARE_BIN)
wasm: $(WASM_PKG)

$(FIRMWARE_BIN): $(FIRMWARE_SRC)
	cd $(FIRMWARE_DIR) && cargo build --release
	cd $(FIRMWARE_DIR) && cargo objcopy --release -- -O binary $(FIRMWARE_BIN)

$(WASM_PKG): $(CONTROL_SRC)
	cd $(CONTROL_DIR) && wasm-pack build --target web --out-dir www/pkg

$(WWW_FIRMWARE): $(FIRMWARE_BIN)
	cp $(FIRMWARE_BIN) $(WWW_FIRMWARE)

serve: $(WASM_PKG) $(WWW_FIRMWARE)
	python3 -m http.server 8080 --directory $(WWW_DIR)

clean:
	cd $(FIRMWARE_DIR) && cargo clean
	cd $(CONTROL_DIR) && cargo clean
	rm -rf $(WWW_DIR)/pkg $(WWW_FIRMWARE)

check:
	./scripts/check.sh

install-hooks:
	ln -sf ../../scripts/pre-push .git/hooks/pre-push
