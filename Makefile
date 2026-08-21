PREFIX ?= $(HOME)/.local
BIN := target/release/mimo-herdr

.PHONY: all build install uninstall verify status test clean

all: build

build:
	cargo build --release

install: build
	mkdir -p $(PREFIX)/bin
	cp $(BIN) $(PREFIX)/bin/mimo-herdr
	$(PREFIX)/bin/mimo-herdr install

uninstall:
	-mimo-herdr uninstall --shim
	rm -f $(PREFIX)/bin/mimo-herdr

verify: build
	$(BIN) verify

status: build
	$(BIN) status

test:
	cargo test

clean:
	cargo clean
